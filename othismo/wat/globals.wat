(module
  ;; Import globals
  (import "env" "global_int" (global $imported_int (mut i32)))
  (import "env" "global_float" (global $imported_float (mut f32)))

  ;; regular globals
  (global $some_int (mut i32) (i32.const 52))
  (global $some_float (mut f32) (f32.const 6.14))

  ;; Export globals 
  (global $exported_int (export "result_int") (mut i32) (i32.const 42))
  (global $exported_float (export "result_float") (mut f32) (f32.const 3.14))

  ;; Export function that increments globals
  (func $increment (export "increment")
    ;; Add 1 to first global
    global.get $imported_int
    i32.const 1
    i32.add
    global.set $imported_int

    ;; Add 2 to second global
    global.get $imported_float
    f32.const 2
    f32.add  
    global.set $imported_float

    ;; Add 3 to third global
    global.get $exported_int
    i32.const 3
    i32.add
    global.set $exported_int

    ;; Add 4 to fourth global
    global.get $exported_float
    f32.const 4
    f32.add
    global.set $exported_float
  )
)