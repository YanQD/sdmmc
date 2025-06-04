pub mod pythium;
pub mod rockship;

extern crate alloc;
use alloc::string::String;

use alloc::vec::Vec;
use core::fmt::Debug;

use super::commands::DataBuffer;
use super::commands::MmcCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmcHostErr {
    CommandError,
    Timeout,
    Unsupported,
    InvalidValue,
    NotReady,
}

pub type MmcHostResult<T = ()> = Result<T, MmcHostErr>;

#[derive(Debug)]
pub struct UDevice {
    pub name: String,
    pub compatible: Vec<String>,
}

pub trait MmcHostOps: Debug + Send + Sync {
    fn send_cmd(&self, cmd: &MmcCommand, data_buffer: Option<DataBuffer>) -> MmcHostResult;
    fn card_busy(&self) -> bool;
    fn set_ios(&self) -> MmcHostResult<()>;
    fn get_cd(&self) -> MmcHostResult<bool>;
}
