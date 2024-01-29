use lazy_static::lazy_static;
use crate::solidarity::{Errors, SolidarityError};
use crate::solidarity::image::ImageFile;

lazy_static! {
    static ref WASM: Vec<u8> = {
        match wasmer::wat2wasm(r#"(module
            (func (export "addTwo") (param i32 i32) (result i32)
                local.get 0
                local.get 1
                i32.add))
        "#.as_bytes()) {
            Ok(bytes) => bytes.to_vec(),
            Err(err) => panic!("Failed to convert WAT to bytes: {:?}", err),
        }
    };
}

#[test]
fn file_can_import_modules() {
    let mut file = ImageFile::create_in_memory().unwrap();

    file.import_module_bytes("/test/module", &WASM).unwrap();

    let modules = file.list_modules().unwrap();

    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0], "/test/module");
}


#[test]
fn file_can_not_import_over_existing_modules() {
    let mut file = ImageFile::create_in_memory().unwrap();

    file.import_module_bytes("/test/module", &WASM).unwrap();
    let result = file.import_module_bytes("/test/module", &WASM);

    assert!(matches!(result, Err(Errors::Solidarity(SolidarityError::ModuleAlreadyExists))));
}


#[test]
fn file_can_delete_modules() {
    let mut file = ImageFile::create_in_memory().unwrap();

    file.import_module_bytes("/test/module", &WASM).unwrap();

    file.remove_object("/test/module").unwrap();

    let modules = file.list_modules().unwrap();

    assert_eq!(modules.len(), 0);
}
