use core::str;
use std::{ffi::c_void, pin::pin};

use libc::size_t;

use micropython::{Vm, VmState};
use micropython_sys as mp;

mod qstr {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(unused)]

    include!(concat!(env!("OUT_DIR"), "/qstr.rs"));
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn mp_hal_stdout_tx_strn_cooked(string: *const u8, len: size_t) {
    unsafe {
        let string = core::slice::from_raw_parts(string, len);
        let Ok(string) = str::from_utf8(&string[..len]) else {
            return;
        };
        print!("{string}");
    }
}

/// Run a garbage collection cycle.
///
/// # Safety
#[no_mangle]
pub unsafe extern "C" fn gc_collect() {
    mp::gc_collect_start();
    mp::gc_helper_collect_regs_and_stack();
    mp::gc_collect_end();
}

/// Called if an exception is raised outside all C exception-catching handlers.
#[no_mangle]
pub extern "C" fn nlr_jump_fail(_val: *const c_void) {
    panic!("nlr jump failure");
}

fn main() {
    let mut state = VmState::<8192>::new();
    let pin = pin!(state);
    let vm = Vm::new(pin);
    let mut obj = vm
        .compile(
            qstr::MP_QSTR__lt_stdin_gt_,
            "print('hello world!', list(x + 1 for x in range(10)), end='eol\\n')",
        )
        .unwrap();
    vm.exec(&mut obj);
    vm.exec(&mut obj);
    vm.exec(&mut obj);
}
