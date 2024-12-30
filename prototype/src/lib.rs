static mut BUFFER: [u8; 5] = *b"HELLO";
static mut HEAP: Option<Box<String>> = None;

#[no_mangle]
pub extern "C" fn increment() -> i32 {
    unsafe {
        BUFFER = *b"NOOPE";
    }

    let heap_ptr = unsafe {
        match HEAP.as_mut() {
            None => {
                HEAP = Some(Box::new("START".to_string()));
            },
            Some(text) => {
                text.push_str("NOPE");
            }
        };

        let s = HEAP.as_ref().unwrap();
        s.as_ptr()
    };

    heap_ptr as i32
}