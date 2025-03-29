use std::{collections::HashMap, mem};

/*
https://blog.rust-lang.org/2024/09/24/webassembly-targets-change-in-default-target-features.html#disabling-on-by-default-webassembly-proposals
https://github.com/WebAssembly/tool-conventions/blob/main/BasicCABI.md
 */

static mut COUNTER: u32 = 0;



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


#[link(wasm_import_module = "othismo")]
extern "C" {
    fn send_message(handle: u64, bytes: *const u8, length: usize) -> u32;
}

#[no_mangle]
pub unsafe extern "C" fn allocate_message(message_length: u32) -> u64 {
    let inbox = inbox();

    let (handle, ptr) = inbox.allocate(message_length as usize);

    (handle.0 as u64) << 32 | ptr as u64
}

#[no_mangle]
pub unsafe extern "C" fn message_received(message_handle: u32) {
    let outbox = outbox();
    let inbox = inbox();
    
    let message = inbox.as_slice(message_handle.into()).expect("The said there was a message in the Inbox, and there wasn't");
    let length = std::cmp::min(COUNTER as usize, message.len());

    let (response_handle, response_ptr) = outbox.assign(message.to_vec());
    
    send_message(response_handle.0 as u64, response_ptr, message.len());
}


#[allow(static_mut_refs)] // wasm is single threaded
fn inbox() -> &'static mut MailBox {
    static mut INBOX: Option<Box<MailBox>> = None;

    unsafe { INBOX.get_or_insert(Box::new(MailBox::default())) }
}

#[allow(static_mut_refs)] // wasm is single threaded
fn outbox() -> &'static mut MailBox {
    static mut OUTBOX: Option<Box<MailBox>> = None;
    
    unsafe { OUTBOX.get_or_insert(Box::new(MailBox::default())) }
}

#[no_mangle]
pub extern "C" fn _othismo_start() {
    unsafe {
        if COUNTER == 0 {
            COUNTER += 3;
        }
    };
}