use std::{marker::PhantomData, pin::Pin};

use micropython_sys as mp_sys;

pub type QStr = u32;

pub struct VmState<const HEAP_SIZE: usize> {
    heap: [u8; HEAP_SIZE],
    stack_top: core::ffi::c_int,
}

impl<const HEAP_SIZE: usize> VmState<HEAP_SIZE> {
    pub fn new() -> Self {
        Self {
            heap: [0u8; HEAP_SIZE],
            stack_top: 0,
        }
    }
}

impl<const HEAP_SIZE: usize> Default for VmState<HEAP_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Vm<'state, const HEAP_SIZE: usize> {
    _state: Pin<&'state mut VmState<HEAP_SIZE>>,
}

impl<'state, const HEAP_SIZE: usize> Vm<'state, HEAP_SIZE> {
    pub fn new(mut state: Pin<&'state mut VmState<HEAP_SIZE>>) -> Self {
        unsafe {
            mp_sys::mp_stack_set_top(&mut state.stack_top as *mut i32 as _);
            let stack_top = &mut state.heap as *mut u8;
            mp_sys::gc_init(stack_top as _, stack_top.add(HEAP_SIZE) as _);
            mp_sys::mp_init();
        }
        Self { _state: state }
    }

    pub fn compile<'vm>(&'vm self, source: QStr, code: &str) -> Option<Object<'vm>> {
        let mut nlr = core::mem::MaybeUninit::uninit();
        let ret = unsafe { mp_sys::nlr_push(nlr.as_mut_ptr()) };
        if ret == 0 {
            unsafe {
                let lex = mp_sys::mp_lexer_new_from_str_len(
                    source as _,
                    code.as_ptr() as _,
                    code.len(),
                    0,
                );
                let source_name = (*lex).source_name;
                let mut parse_tree =
                    mp_sys::mp_parse(lex, mp_sys::mp_parse_input_kind_t_MP_PARSE_FILE_INPUT);
                let object = mp_sys::mp_compile(&mut parse_tree as _, source_name, true);
                mp_sys::nlr_pop();
                Some(Object {
                    object,
                    _phantom: PhantomData,
                })
            }
        } else {
            let nlr = unsafe { nlr.assume_init() };
            unsafe {
                mp_sys::mp_obj_print_exception(&mp_sys::mp_plat_print as _, nlr.ret_val as _)
            };
            None
        }
    }

    pub fn exec<'vm>(&'vm self, object: &mut Object<'vm>) {
        let mut nlr = core::mem::MaybeUninit::uninit();
        let ret = unsafe { mp_sys::nlr_push(nlr.as_mut_ptr()) };
        if ret == 0 {
            unsafe { mp_sys::mp_call_function_0(object.object) };
        } else {
            let nlr = unsafe { nlr.assume_init() };
            unsafe {
                mp_sys::mp_obj_print_exception(&mp_sys::mp_plat_print as _, nlr.ret_val as _)
            };
        }
    }
}

impl<'state, const HEAP_SIZE: usize> Drop for Vm<'state, HEAP_SIZE> {
    fn drop(&mut self) {
        unsafe {
            mp_sys::mp_deinit();
        }
    }
}

pub struct Object<'vm> {
    object: mp_sys::mp_obj_t,
    _phantom: core::marker::PhantomData<&'vm ()>,
}
