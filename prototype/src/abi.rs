use crate::tasks::executor;
use std::{collections::HashMap, future::Future, task::Waker};

/*
https://blog.rust-lang.org/2024/09/24/webassembly-targets-change-in-default-target-features.html#disabling-on-by-default-webassembly-proposals
https://github.com/WebAssembly/tool-conventions/blob/main/BasicCABI.md
 */

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "othismo")]
extern "C" {
    fn _send_message(bytes: *mut u8, length: u32) -> u32;
}

#[no_mangle]
#[cfg(not(target_arch = "wasm32"))]
pub unsafe extern "C" fn _send_message(bytes: *mut u8, length: u32) -> u32 {
    let slice = std::slice::from_raw_parts_mut(bytes, length as usize);

    let (handle, _) = tests::sent_test_messages().assign(slice.into());

    handle.0
}

#[no_mangle]
pub unsafe extern "C" fn _allocate_message(message_length: u32) -> *const u8 {
    let inbox = inbox();

    inbox.allocate(message_length as usize)
}

#[no_mangle]
pub unsafe extern "C" fn _run() {
    loop {
        if !executor().try_tick() {
            break;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn _message_received(message_handle: u32) {
    let inbox = inbox();
    let executor = executor();

    if let Some(message) = inbox.take(message_handle.into()) {
        // TODO extract request id if any; and wake up the relevant future
        executor.spawn(process_message(message)).detach();
    }
}

pub fn send_message(mut message: Vec<u8>) -> impl Future<Output = Vec<u8>> {
    let ptr = message.as_mut_ptr();
    let len = message.len() as u32;

    let handle = unsafe { _send_message(ptr, len) };

    let task = ReceiveResponseTask {
        request: handle.into(),
        response: None,
    };

    executor().spawn(task)
}

async fn process_message(message: Vec<u8>) {
    let a = send_message(message.clone());
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
fn inflight_requests() -> &'static mut HashMap<MessageHandle, Waker> {
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

struct MailBox {
    buffers: HashMap<MessageHandle, Vec<u8>>,
}

impl MailBox {
    pub fn allocate(&mut self, length: usize) -> *const u8 {
        let mut buffer = Vec::new();
        buffer.resize(length, 0);

        let ptr = buffer.as_ptr();
        let handle = MessageHandle(ptr as u32);

        self.buffers.insert(handle, buffer);

        ptr
    }

    pub fn assign(&mut self, message: Vec<u8>) -> (MessageHandle, *const u8) {
        let ptr = message.as_ptr();
        let handle = MessageHandle(ptr as u32);

        self.buffers.insert(handle, message);

        (handle, ptr)
    }

    pub fn as_slice(&self, handle: MessageHandle) -> Option<&[u8]> {
        self.buffers.get(&handle).map(|v| v.as_slice())
    }

    pub fn take(&mut self, handle: MessageHandle) -> Option<Vec<u8>> {
        self.buffers.remove(&handle)
    }
}

impl Default for MailBox {
    fn default() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }
}

struct ReceiveResponseTask {
    request: MessageHandle,
    response: Option<MessageHandle>,
}

impl Future for ReceiveResponseTask {
    type Output = Vec<u8>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        inflight_requests().insert(self.request, cx.waker().clone());

        match inbox().take(self.request) {
            Some(response) => std::task::Poll::Ready(response),
            None => std::task::Poll::Pending,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use crate::abi::MailBox;

    #[allow(static_mut_refs)] // wasm is single threaded
    pub(crate) fn sent_test_messages() -> &'static mut MailBox {
        static mut TEST_OUTBOX: Option<Box<MailBox>> = None;

        unsafe { TEST_OUTBOX.get_or_insert(Box::new(MailBox::default())) }
    }

    #[test]
    fn two_messages_are_echoed_back() {
        let message = b"Hello";
        inject_message(message);
        run_until_idle();

        assert!(sent_test_messages().buffers.len() == 2);
        assert_eq!(sent_test_messages().take(0.into()), Some(b"Hello".into()));
        assert_eq!(sent_test_messages().take(1.into()), Some(b"Hello".into()));
    }

    #[cfg(test)]
    fn inject_message(message: &[u8]) {
        unsafe {
            let ptr = _allocate_message(message.len() as u32) as *mut u8;
            std::ptr::copy_nonoverlapping(message.as_ptr(), ptr, message.len());
            _message_received(0);
        };
    }

    #[cfg(test)]
    fn inject_response(message: &[u8], request: MessageHandle) {
        unsafe {
            let ptr = _allocate_message(message.len() as u32) as *mut u8;
            std::ptr::copy_nonoverlapping(message.as_ptr(), ptr, message.len());
            _message_received(request.0);
        };
    }

    #[cfg(test)]
    fn run_until_idle() {
        unsafe {
            _run();
        }
    }
}
