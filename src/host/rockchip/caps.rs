use crate::common::HostCapabilities;

#[derive(Debug, Clone)]
pub struct SdhciCapabilities {
    pub voltages: u32,
    pub host_caps: u32,
    pub clock_base: u32,
    pub version: u16,
    pub quirks: u32,
}

impl HostCapabilities for SdhciCapabilities {
    fn get_voltages(&self) -> u32 {
        self.voltages
    }

    fn get_host_caps(&self) -> u32 {
        self.host_caps
    }

    fn get_clock_base(&self) -> u32 {
        self.clock_base
    }

    fn get_version(&self) -> u16 {
        self.version
    }

    fn get_quirks(&self) -> u32 {
        self.quirks
    }
}
