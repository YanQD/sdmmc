use core::time::Duration;

use crate::{
    constants::*,
    host::{MmcHostError, MmcHostResult, rockchip::SdhciHost},
    impl_register_ops, mci_sleep,
};

impl_register_ops!(SdhciHost, base_addr);

// operation to control the host controller
impl SdhciHost {
    // Reset the controller
    #[inline]
    pub fn reset(&self, reset_flag: u8) -> MmcHostResult {
        // Request reset
        self.write_reg8(EMMC_SOFTWARE_RESET, reset_flag);

        // Wait for reset to complete with timeout
        let mut timeout = 20; // Increased timeout
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & reset_flag) != 0 {
            if timeout == 0 {
                return Err(MmcHostError::Timeout);
            }
            timeout -= 1;
            mci_sleep(Duration::from_micros(1000));
        }

        Ok(())
    }
}
