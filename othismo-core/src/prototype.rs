use std::{any::type_name_of_val, fs::File, io::{BufReader, BufWriter}};

use wasmbin::sections::{payload, Data, DataInit};
use wasmer::Store;

use crate::othismo::{Result, OthismoError};

pub fn dehydrate_instance(module_name: &str, instance_name: &str, instance: &wasmer::Instance) -> Result<()> {
    let store = Store::default();
    let mut module = wasmbin::Module::decode_from(BufReader::new(std::fs::File::open(&module_name)?))?;

    let datas = module.find_or_insert_std_section(|| payload::Data::default()).try_contents_mut()?;
    datas.clear();
    
    let memory = instance.exports.get_memory("memory")?;
    let view = memory.view(&store);
    println!("{:?}", view.size());

    module.encode_into(BufWriter::new(File::create("output.wasm")?))?;
    Ok(())
} 

pub fn hydrate_instance(module: &wasmer::Module, name: &str) -> Result<wasmer::Instance> { unimplemented!() }

pub fn parse_module_2(module_path: String) -> Result<()> {
    let mut module = wasmbin::Module::decode_from(BufReader::new(std::fs::File::open(&module_path)?))?;

    let datas = module.find_or_insert_std_section(|| payload::Data::default()).try_contents_mut()?;
    datas.clear();
    datas.push(Data {
        blob: Vec::new(),
        init: DataInit::Passive
    });
    
    module.encode_into(BufWriter::new(File::create("output.wasm")?))?;
    Ok(())
}