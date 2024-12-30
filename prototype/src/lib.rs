static mut COUNTER: Option<Box<i32>> = None;


#[no_mangle]
pub extern "C" fn increment() -> i32 {
    let counter_ref = unsafe { COUNTER.as_mut() };

    if let Some(counter) = counter_ref {
        **counter += 1;
    } else {
        unsafe {
            COUNTER = Some(Box::new(0xDEAD))
        };
    }

    let count = unsafe { COUNTER.as_mut().unwrap() };
    **count
}