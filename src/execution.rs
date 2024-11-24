use std::io::BufWriter;

use wasmer::{imports, Global, Instance, Store, TypedFunction, Value};

use crate::solidarity::image::{Image, InstanceAtRest, Object};
use crate::solidarity::{Errors, Result, SolidarityError};

struct Session<'s> {
    image: &'s mut Image,
    store: Store,
}

struct InstanceSession {
    globals: Vec<(Vec<String>, Global)>,
    module: InstanceAtRest,
    instance: Instance
}

impl InstanceSession {
    pub fn from_instance_at_rest(store: &mut Store, instance_at_rest: InstanceAtRest) -> Result<InstanceSession> {
        let globals = vec![
            (vec!["env".to_string(), "g".to_string()], Global::new_mut(store, Value::I32(42))),
        ];
        let environment = imports! {
            "env" => {
                "g" => globals[0].1.clone(),
            }
        };

        let buffer = instance_at_rest.to_bytes();
        let wasmer_instance_module = wasmer::Module::new(store, &buffer)?;
        let wasmer_instance = wasmer::Instance::new(store, &wasmer_instance_module, &environment)?;

        Ok(InstanceSession {
            globals: globals,
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

    pub fn print_globals(&self, store: &mut Store) {
        for global in self.globals.iter() {
            println!("{:?}", global.1.get(store))
        }
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

    instance_session.print_globals(&mut store);
    instance_session.call_function(&mut store)?;
    instance_session.print_globals(&mut store);
    
    let mut dehydrated_instance = InstanceAtRest::from(instance_session);
    image.remove_object(instance_name)?;
    image.import_object(instance_name, Object::Instance(dehydrated_instance))?;
    Ok(())
}
