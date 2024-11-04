use std::io::BufWriter;

use wasmer::{imports, Global, Store, Value};

use crate::solidarity::image::{Image, Object};
use crate::solidarity::{Errors, Result, SolidarityError};

struct Session<'s> {
    image: &'s mut Image
}

pub fn send_message(image: &mut Image, instance_name: &str) -> Result<()> {
    let object = image.get_object(instance_name)?;
    let instance_module = match object {
        Object::Instance(instance) => instance,
        Object::Module(_) => Err(SolidarityError::ObjectDoesNotExist)?
    };

    let mut store = Store::default();
    let wasmer_module = from_wasmbin_to_wasmer_module(&store, &instance_module)?;
    let global = Global::new_mut(&mut store, Value::I32(0));
    let environment = imports! {
        "env" => {
            "g" => global.clone()
        }
    };
    let instance = wasmer::Instance::new(&mut store, &wasmer_module, &environment)?;


    println!("global before {:?}", &global.get(&mut store)); 
    instance.exports.get_function("f").unwrap().call(&mut store, &[]);
    println!("global after {:?}", &global.get(&mut store)); 
    
    Ok(())
}

fn from_wasmbin_to_wasmer_module(store: &Store, wasmbin: &wasmbin::Module) -> Result<wasmer::Module> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut writer = BufWriter::new(&mut buffer);

    wasmbin.encode_into(writer)?;
    Ok(wasmer::Module::new(store, &buffer)?)
}