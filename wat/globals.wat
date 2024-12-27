(module
  ;; Import globals
  (import "env" "global_int" (global $imported_int i32))
  (import "env" "global_float" (global $imported_float f32))

  ;; Export globals
  (global $exported_int (export "result_int") i32 (i32.const 42))
  (global $exported_float (export "result_float") f32 (f32.const 3.14))

  ;; Export function
  (func $do_nothing (export "do_nothing"))
)