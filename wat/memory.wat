(module
  ;; Be explicit about min size (1 page) and optionally max
  (memory $memory0 (import "env" "mem1") 1)  ;; or (memory $memory0 (import "env" "mem1") 1 1) for fixed size
  
  ;; Data segments
  (data (i32.const 256) "Hello, WebAssembly!\00")  ;; Ends at ~275
  (data (i32.const 512) "\01\02\03\04\05")        ;; Ends at ~517
)