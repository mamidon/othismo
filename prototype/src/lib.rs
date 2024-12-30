static mut COUNTER: Option<Box<i32>> = None;

#[no_mangle]
pub extern "C" fn start() {
    unsafe {
        COUNTER = Some(Box::new(0));
    }
}

#[no_mangle]
pub extern "C" fn increment() -> i32 {
    unsafe {
        let counter = COUNTER.as_mut().unwrap();
        **counter += 1;
        **counter
    }
}