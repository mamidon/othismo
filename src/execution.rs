use std::io::BufWriter;

use wasmer::{imports, Global, Instance, Store, TypedFunction, Value};

use crate::solidarity::image::{Image, Object};
use crate::solidarity::{Errors, Result, SolidarityError};

struct Session<'s> {
    image: &'s mut Image,
    store: Store,
}

struct InstanceSession {
    globals: Vec<Global>,
    instance: Instance
}

impl InstanceSession {
    pub fn from_wasmbin_module(store: &mut Store, wasmbin_instance_module: &wasmbin::Module) -> Result<InstanceSession> {
        let mut buffer: Vec<u8> = Vec::new();
        let mut writer = BufWriter::new(&mut buffer);
        wasmbin_instance_module.encode_into(writer)?;

        let globals = vec![
            Global::new(store, Value::F32(42.0)),
            Global::new_mut(store, Value::F32(32.0))
        ];
        let environment = imports! {
            "env" => {
                "some" => globals[0].clone(),
                "other" => globals[1].clone(),
            }
        };

        let wasmer_instance_module = wasmer::Module::new(store, &buffer)?;
        let wasmer_instance = wasmer::Instance::new(store, &wasmer_instance_module, &environment)?;

        Ok(InstanceSession {
            globals: globals,
            instance: wasmer_instance
        })
    }

    pub fn call_function(&self, store: &mut Store) -> Result<()> {
        let set_some: TypedFunction<(f32), ()> = self.instance
        .exports
        .get_function("set_other")?
        .typed(store)?;

        set_some.call(store, 100.0)?;
        Ok(())
    }

    pub fn print_globals(&self, store: &mut Store) {
        for global in self.globals.iter() {
            println!("{:?}", global.get(store))
        }
    }
}

pub fn send_message(image: &mut Image, instance_name: &str) -> Result<()> {
    let object = image.get_object(instance_name)?;
    let instance_module = match object {
        Object::Instance(instance) => instance,
        Object::Module(_) => Err(SolidarityError::ObjectDoesNotExist)?
    };

    let mut store = Store::default();
    let instance_session = InstanceSession::from_wasmbin_module(&mut store, &instance_module)?;


    instance_session.print_globals(&mut store);
    instance_session.call_function(&mut store)?;
    instance_session.print_globals(&mut store);
    Ok(())
}

fn from_wasmbin_to_wasmer_module(store: &Store, wasmbin: &wasmbin::Module) -> Result<wasmer::Module> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut writer = BufWriter::new(&mut buffer);

    wasmbin.encode_into(writer)?;
    Ok(wasmer::Module::new(store, &buffer)?)
}