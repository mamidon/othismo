use core::hash;
use std::{cell::RefCell, collections::HashMap, future::Future, mem, task::Waker};
use crate::tasks::executor;

/*
https://blog.rust-lang.org/2024/09/24/webassembly-targets-change-in-default-target-features.html#disabling-on-by-default-webassembly-proposals
https://github.com/WebAssembly/tool-conventions/blob/main/BasicCABI.md
 */

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "othismo")]
extern "C" {
    fn _send_message(handle: u32, bytes: *const u8, length: usize) -> u32;
    fn _cast_message(handle: u32, bytes: *const u8, length: usize) -> u32;
}


#[no_mangle]
#[cfg(not(target_arch = "wasm32"))]
pub extern "C" fn _send_message(handle: u32, bytes: *const u8, length: usize) -> u32 {
    0
}

#[no_mangle]
#[cfg(not(target_arch = "wasm32"))]
pub extern "C" fn _cast_message(handle: u32, bytes: *const u8, length: usize) -> u32 {
    0
}


#[no_mangle]
pub extern "C" fn _othismo_start() {

}

#[no_mangle]
pub unsafe extern "C" fn _allocate_message(message_length: u32) -> u64 {
    let inbox = inbox();

    let (handle, ptr) = inbox.allocate(message_length as usize);

    (handle.0 as u64) << 32 | ptr as u64
}

#[no_mangle]
pub unsafe extern "C" fn _run() {
    let executor = executor();

    while (executor.try_tick()) {

    }

}

#[no_mangle]
pub unsafe extern "C" fn _message_received(message_handle: u32, in_response_to_handle: u32) {
    let inbox = inbox();
    let executor = executor();

    match in_response_to_handle {
        0 => {
            executor.spawn(process_message(inbox.as_slice(message_handle.into()).expect("They said a message arrived"))).detach();
        },
        handle => {
            match outbox().get(&handle.into()) {
                Some(waker) => {
                    waker.wake_by_ref();
                },
                None => {}
            }
        }
    }

    let mut x = 0;
    while (executor.try_tick()) {
        x += 1;
    }
}

pub fn send_message(message: &[u8]) -> impl Future<Output = ()> {
    let handle = MessageHandle(outbox().len() as u32);

    let task = ReceiveResponseTask {
        request: handle,
    };

    let spawned_task = executor().spawn(task);

    unsafe { _send_message(handle.0, message.as_ptr(), message.len()); }

    spawned_task
}

async fn process_message(message: &[u8]) {
    let a = send_message(message);
    let b = send_message(message);

    a.await;
    b.await;
}

#[allow(static_mut_refs)] // wasm is single threaded
fn inbox() -> &'static mut MailBox {
    static mut INBOX: Option<Box<MailBox>> = None;

    unsafe { INBOX.get_or_insert(Box::new(MailBox::default())) }
}

#[allow(static_mut_refs)] // wasm is single threaded
fn outbox() -> &'static mut HashMap<MessageHandle, Waker> {
    static mut OUTBOX: Option<HashMap<MessageHandle, Waker>> = None;
    
    unsafe { OUTBOX.get_or_insert(HashMap::new()) }
}


#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct MessageHandle(u32);

impl From<u32> for MessageHandle {
    fn from(value: u32) -> Self {
        MessageHandle(value)
    }
}

struct VolatileBuffer {
    ptr: *const u8,
    len: usize
}

impl VolatileBuffer {
    pub fn empty(len: usize) -> VolatileBuffer {
        let mut buffer = Vec::with_capacity(len);
        let ptr = buffer.as_mut_ptr();
        mem::forget(buffer);
        VolatileBuffer { 
            ptr, 
            len
        }
    }

    pub fn from_bytes(mut bytes: Vec<u8>) -> VolatileBuffer {
        let len = bytes.len();
        let ptr = bytes.as_mut_ptr();
        mem::forget(bytes);
        VolatileBuffer { 
            ptr, 
            len
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }
}

struct MailBox {
    next_handle: MessageHandle,
    buffers: HashMap<MessageHandle, VolatileBuffer>
}

impl MailBox {

    pub fn allocate(&mut self, length: usize) -> (MessageHandle, *const u8) {
        let handle = self.take_next_handle();
        let buffer = VolatileBuffer::empty(length);
        let ptr = buffer.ptr;

        self.buffers.insert(handle, buffer);

        (handle, ptr)
    }

    pub fn assign(&mut self, message: Vec<u8>) -> (MessageHandle, *const u8) {
        let handle = self.take_next_handle();
        let buffer = VolatileBuffer::from_bytes(message);
        let ptr = buffer.as_ptr();

        self.buffers.insert(handle, buffer);

        (handle, ptr)
    }

    pub fn as_slice(&self, handle: MessageHandle) -> Option<&[u8]> {
        self.buffers.get(&handle).map(|v| v.as_slice())
    }

    fn take_next_handle(&mut self) -> MessageHandle {
        let handle = self.next_handle;
        self.next_handle = MessageHandle(self.next_handle.0 + 1);
        handle
    }
}

impl Default for MailBox {
    fn default() -> Self {        
        Self {
            buffers: HashMap::new(),
            next_handle: 0.into()
        }
    }
}

struct ReceiveResponseTask {
    request: MessageHandle,
}

impl Future for ReceiveResponseTask {
    type Output = ();

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        outbox().insert(self.request, cx.waker().clone());

        match inbox().as_slice(self.request) {
            Some(_) => std::task::Poll::Ready(()),
            None => std::task::Poll::Pending,
        }
    }
}