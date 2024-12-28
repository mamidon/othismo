use std::io::BufWriter;

use wasmer::{imports, Global, Imports, Instance, Store, TypedFunction, Value};

use crate::solidarity::image::{GlobalMutability, Image, InstanceAtRest, Object};
use crate::solidarity::{Errors, Result, SolidarityError};

struct Session<'s> {
    image: &'s mut Image,
    store: Store,
}

struct InstanceSession {
    module: InstanceAtRest,
    instance: Instance
}

impl InstanceSession {
    pub fn from_instance_at_rest(store: &mut Store, instance_at_rest: InstanceAtRest) -> Result<InstanceSession> {
        let buffer = instance_at_rest.to_bytes();
        let wasmer_instance_module = wasmer::Module::new(store, &buffer)?;
        let wasmer_instance = wasmer::Instance::new(store, &wasmer_instance_module, &imports! {})?;

        Ok(InstanceSession {
            module: instance_at_rest,
            instance: wasmer_instance
        })
    }

    pub fn into_instance_at_rest(mut self, store: &mut Store) -> Result<InstanceAtRest> {
        for (name, value) in self.instance.exports {
            match value {
                wasmer::Extern::Global(global) => self.module.set_export(&name, global.get(store))?,
                _ => continue
            }
        }

        Ok(self.module)
    }

    pub fn call_function(&self, store: &mut Store) -> Result<()> {
        let set_some: TypedFunction<(), ()> = self.instance
        .exports
        .get_function("increment")?
        .typed(store)?;

        set_some.call(store)?;
        Ok(())
    }
}

pub fn send_message(image: &mut Image, instance_name: &str) -> Result<()> {
    let object = image.get_object(instance_name)?;
    let instance_at_rest = match object {
        Object::Instance(instance_at_rest) => instance_at_rest,
        Object::Module(_) => Err(SolidarityError::ObjectDoesNotExist)?
    };

    let mut store = Store::default();
    let instance_session = InstanceSession::from_instance_at_rest(
        &mut store, 
        instance_at_rest
    )?;

    instance_session.call_function(&mut store)?;
    
    let mut dehydrated_instance = instance_session.into_instance_at_rest(&mut store)?;
    image.remove_object(instance_name)?;
    image.import_object(instance_name, Object::Instance(dehydrated_instance))?;
    Ok(())
}
