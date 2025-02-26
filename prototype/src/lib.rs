
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
static mut COUNTER: u32 = 0;

#[no_mangle]
pub extern "C" fn _othismo_start() {
    unsafe {
        if COUNTER == 0 {
            COUNTER += 3;
        }
    };
}

struct MailBox {
    buffer: Vec<u8>
}

impl MailBox {
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buffer.as_mut_slice()
    }

    pub fn take(&mut self) -> Vec<u8> {
        std::mem::replace(&mut self.buffer, Vec::with_capacity(1024))
    }

    pub fn resize(&mut self, required_capacity: usize) {
        if self.buffer.len() >= required_capacity {
            return;
        }

        self.buffer.resize(required_capacity, 0u8);
    }
}

impl Default for MailBox {
    fn default() -> Self {
        let initial_capacity = 1024;
        
        Self {
            buffer: Vec::with_capacity(initial_capacity)
        }
    }
}

#[link(wasm_import_module = "othismo")]
extern "C" {
    fn send_message(bytes: *const u8, length: usize) -> u32;
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
    
    let message = inbox.take();
    
    let length = std::cmp::min(COUNTER as usize, message.len());
    outbox.resize(length);
    outbox.as_mut_slice().copy_from_slice(&message.as_slice()[..length]);

    let outbox_slice = outbox.as_slice();
    send_message(outbox_slice.as_ptr(), outbox_slice.len());
}