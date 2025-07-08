#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

#[macro_use]
pub mod regs;
pub mod mci;
pub mod mci_host;
pub mod osa;

mod tools;

use alloc::{format, vec::Vec};
use log::error;
pub use mci_host::*;

// mod aux;
// mod card;
// mod common;

// pub mod constants;
// pub mod host;
// pub mod mci_core;

use core::time::Duration;

// pub use crate::common::clock::*;
pub const BLOCK_SIZE: usize = 512;

pub unsafe fn dump_memory_region(addr: usize, size: usize) {
    let start_ptr: *const u32 = addr as *const u32;
    let word_count = size / 4;

    error!("Memory dump from 0x{:08x}:", addr);

    for chunk_start in (0..word_count).step_by(8) {
        let mut values = Vec::new();
        let chunk_end = (chunk_start + 8).min(word_count);

        for i in chunk_start..chunk_end {
            let value = unsafe { *start_ptr.add(i) };
            values.push(format!("{:08x}", value));
        }

        error!("  0x{:08x}: [{}]", addr + chunk_start * 4, values.join(" "));
    }
}

pub trait Kernel {
    fn sleep(duration: Duration);
}

pub(crate) fn mci_sleep(duration: Duration) {
    unsafe extern "Rust" {
        fn _mci_sleep(duration: Duration);
    }

    unsafe {
        _mci_sleep(duration);
    }
}

#[macro_export]
macro_rules! set_impl {
    ($t: ty) => {
        #[unsafe(no_mangle)]
        unsafe fn _mci_sleep(duration: Duration) {
            <$t as $crate::Kernel>::sleep(duration)
        }
    };
}
