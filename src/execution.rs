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
        let state = instance_at_rest.find_or_create_state()?;

        let mut environment = Imports::new();

        for (path, global_at_rest) in state.globals.iter() {
            let mut parts = path.split(".");
            let namespace = parts.nth(0).unwrap();
            let name = parts.nth(0).unwrap();

            let global = match global_at_rest.mutability() {
                GlobalMutability::Const => wasmer::Global::new(store, global_at_rest.into()),
                GlobalMutability::Var => wasmer::Global::new_mut(store, global_at_rest.into()),
            };
            
            environment.define(&namespace, &name, wasmer::Extern::Global(global));
        }

        let buffer = instance_at_rest.to_bytes();
        let wasmer_instance_module = wasmer::Module::new(store, &buffer)?;
        let wasmer_instance = wasmer::Instance::new(store, &wasmer_instance_module, &environment)?;

        Ok(InstanceSession {
            module: instance_at_rest,
            instance: wasmer_instance
        })
    }

    pub fn call_function(&self, store: &mut Store) -> Result<()> {
        let set_some: TypedFunction<(), ()> = self.instance
        .exports
        .get_function("f")?
        .typed(store)?;

        set_some.call(store)?;
        Ok(())
    }
}

impl From<InstanceSession> for InstanceAtRest {
    fn from(value: InstanceSession) -> Self {
        value.module
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
    
    let mut dehydrated_instance = InstanceAtRest::from(instance_session);
    image.remove_object(instance_name)?;
    image.import_object(instance_name, Object::Instance(dehydrated_instance))?;
    Ok(())
}
