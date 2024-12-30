use std::io::BufWriter;

use wasmer::{imports, Global, Imports, Instance, Store, TypedFunction, Value};

use crate::othismo::image::{Image, InstanceAtRest, Object};
use crate::othismo::{Errors, Result, OthismoError};

struct Session<'s> {
    image: &'s mut Image,
    store: Store,
}

struct InstanceSession {
    instance_at_rest: InstanceAtRest,
    instance: Instance
}

impl InstanceSession {
    pub fn from_instance_at_rest(store: &mut Store, instance_at_rest: InstanceAtRest) -> Result<InstanceSession> {
        let buffer = instance_at_rest.to_bytes();
        let wasmer_instance_module = wasmer::Module::new(store, &buffer)?;
        let wasmer_instance = wasmer::Instance::new(store, &wasmer_instance_module, &imports! {})?;

        Ok(InstanceSession {
            instance_at_rest,
            instance: wasmer_instance
        })
    }

    pub fn into_instance_at_rest(mut self, store: &mut Store) -> Result<InstanceAtRest> {
        for (name, value) in self.instance.exports {
            if let wasmer::Extern::Global(global) = &value {
                self.instance_at_rest.set_exported_global(&name, global.get(store))?;
            }

            if let wasmer::Extern::Memory(memory) = &value {
                self.instance_at_rest.clear_data_segments();                
                
                let page_size_in_bytes = 1024;
                let view = memory.view(store);
                let mut buffer: Vec<u8> = Vec::with_capacity(page_size_in_bytes);

                for offset in 0..(view.data_size() / page_size_in_bytes as u64) {
                    view.read(offset, &mut buffer);

                    if (buffer.iter().all(|&byte| byte == 0)) {
                        continue;
                    }

                    self.instance_at_rest.add_data_segment(offset as i32, &buffer);
                }
            }
        }
        

        Ok(self.instance_at_rest)
    }

    pub fn call_init(&self, store: &mut Store) -> Result<()> {
        let init: TypedFunction<(), ()> = self.instance.exports.get_function("init")?.typed(store)?;
        init.call(store)?;

        Ok(())
    }

    pub fn call_function(&self, store: &mut Store) -> Result<()> {
        let set_some: TypedFunction<(), (i32)> = self.instance
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
        Object::Module(_) => Err(OthismoError::ObjectDoesNotExist)?
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