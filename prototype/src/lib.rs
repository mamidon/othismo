
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

struct MailBox {
    head: *mut u8,
    length: usize,
    capacity: usize
}

impl MailBox {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.head, self.length) }
    }

    pub fn as_mut_slice(&self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.head, self.length) }
    }

    pub fn resize(&mut self, required_capacity: usize) {
        self.length = required_capacity;

        if self.capacity >= required_capacity {
            return;
        }

        let mut buffer = unsafe { Vec::from_raw_parts(self.head, self.length, self.capacity) };
        buffer.resize(required_capacity, 0u8);

        self.head = buffer.as_mut_ptr();
        std::mem::forget(buffer);
    }
}

impl Default for MailBox {
    fn default() -> Self {
        let initial_capacity = 1024;
        let mut buffer = Vec::with_capacity(initial_capacity);
        let ptr = buffer.as_mut_ptr();
        std::mem::forget(buffer);

        Self {
            head: ptr,
            length: 0,
            capacity: initial_capacity
        }
    }
}

impl Drop for MailBox {
    fn drop(&mut self) {
        let _ = unsafe { Vec::from_raw_parts(self.head, self.length, self.capacity) };
    }
}

#[link(wasm_import_module = "othismo")]
unsafe extern "C" {
    unsafe fn send_message(bytes: *const u8, length: usize);
}

#[no_mangle]
pub unsafe extern "C" fn prepare_inbox(message_length: usize) -> *mut u8 {
    let inbox = inbox();

    inbox.resize(message_length);

    inbox.as_mut_slice().as_mut_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn message_received() {
    let outbox = outbox();
    let inbox = inbox();
    
    let message = inbox.as_slice();

    outbox.resize(message.len());
    outbox.as_mut_slice().copy_from_slice(message);

    let outbox_slice = outbox.as_slice();
    send_message(outbox_slice.as_ptr(), outbox_slice.len());
}