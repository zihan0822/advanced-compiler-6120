#![feature(allocator_api)]

use lazy_static::lazy_static;
use std::alloc::{GlobalAlloc, System};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;

#[derive(Debug)]
struct FuncAllocStat {
    num_alloc: usize,
    total_alloc_size: usize,
}

lazy_static! {
    static ref HeapFootPrint: Mutex<HashMap<String, FuncAllocStat>> = Mutex::default();
}

fn update_func_heap_record(func_name: &str, size: usize) {
    HeapFootPrint
        .lock()
        .unwrap()
        .entry(func_name.to_string())
        .and_modify(|stat| {
            stat.num_alloc += 1;
            stat.total_alloc_size += size
        })
        .or_insert(FuncAllocStat {
            num_alloc: 1,
            total_alloc_size: size,
        });
}

#[unsafe(no_mangle)]
pub extern "C" fn __wrapped_rust_malloc(size: usize, func_name: *const c_char) -> *mut c_void {
    let align = size.next_power_of_two();
    unsafe {
        let func_name = CStr::from_ptr(func_name).to_str().unwrap();
        update_func_heap_record(func_name, size);
        let layout = std::alloc::Layout::from_size_align_unchecked(size, align);
        let allotted = System.alloc(layout) as *mut _;
        allotted
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __wrapped_rust_free(ptr: *const (), _func_name: *mut c_char) {
    unsafe {
        System.dealloc(ptr as *mut _, std::alloc::Layout::new::<()>());
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __heap_alloc_profile() {
    for (func_name, stat) in HeapFootPrint.lock().unwrap().iter() {
        eprintln!("{func_name}: {stat:#?}");
    }
}
