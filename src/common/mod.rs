pub mod clock;
pub(crate) mod commands;
pub(crate) mod regs;

#[allow(dead_code)]
pub trait HostCapabilities: Clone + Send + Sync {
    fn get_voltages(&self) -> u32;
    fn get_host_caps(&self) -> u32;
    fn get_clock_base(&self) -> u32;
    fn get_version(&self) -> u16;
    fn get_quirks(&self) -> u32;
}
