use crate::{
    constants::*,
    delay_us,
    host::rockship::{SdhciError, SdhciHost, SdhciResult},
};
use core::sync::atomic::{Ordering, fence};
use log::{info, warn};

impl SdhciHost {
    /// Write data to SD card buffer register
    /// This is a lower-level function used by data transfer operations
    pub fn write_buffer(&self, buffer: &[u8]) -> SdhciResult {
        // Wait until space is available in the controller buffer
        self.wait_for_interrupt(EMMC_INT_SPACE_AVAIL, 100)?;

        let len = buffer.len();
        info!("Writing {} bytes to buffer", len);
        // Write data in 4-byte chunks
        for i in (0..len).step_by(4) {
            // Pack bytes into a 32-bit word, handling potential buffer underrun
            let mut val: u32 = (buffer[i] as u32) << 0;

            if i + 1 < len {
                val |= (buffer[i + 1] as u32) << 8;
            }

            if i + 2 < len {
                val |= (buffer[i + 2] as u32) << 16;
            }

            if i + 3 < len {
                val |= (buffer[i + 3] as u32) << 24;
            }

            // Ensure all memory operations complete before hardware access
            fence(Ordering::Release);

            // Write the 32-bit word to the buffer data register
            self.write_reg32(EMMC_BUF_DATA, val);
        }

        // Wait for data transfer to complete
        self.wait_for_interrupt(EMMC_INT_DATA_END, 100)?;

        Ok(())
    }

    /// Read data from SD card buffer register
    /// This is a lower-level function used by data transfer operations
    pub fn read_buffer(&self, buffer: &mut [u8]) -> SdhciResult {
        // Wait until data is available in the controller buffer
        self.wait_for_interrupt(EMMC_INT_DATA_AVAIL, 100)?;

        // Read data into buffer in 4-byte chunks
        let len = buffer.len();
        info!("Reading {} bytes into buffer", len);
        for i in (0..len).step_by(4) {
            // Read 32-bit word from buffer data register
            let val = self.read_reg32(EMMC_BUF_DATA);
            delay_us(100);

            // Unpack the 32-bit word into individual bytes, handling buffer boundary
            buffer[i] = (val & 0xFF) as u8;

            if i + 1 < len {
                buffer[i + 1] = ((val >> 8) & 0xFF) as u8;
            }

            if i + 2 < len {
                buffer[i + 2] = ((val >> 16) & 0xFF) as u8;
            }

            if i + 3 < len {
                buffer[i + 3] = ((val >> 24) & 0xFF) as u8;
            }
        }

        // Wait for data transfer to complete
        self.wait_for_interrupt(EMMC_INT_DATA_END, 100)?;

        Ok(())
    }

    /// Wait for a specific interrupt flag to be set
    /// Helper function used by data transfer operations
    /// Parameters:
    /// - flag: The interrupt flag to wait for
    /// - timeout_count: Maximum number of iterations to wait
    fn wait_for_interrupt(&self, flag: u32, timeout_count: u32) -> SdhciResult {
        for _ in 0..timeout_count {
            // Read the current interrupt status
            let int_status = self.read_reg32(EMMC_NORMAL_INT_STAT);

            // Check if the target flag is set
            if int_status & flag != 0 {
                // Clear the flag by writing back to the register (修复：使用32位写入)
                self.write_reg32(EMMC_NORMAL_INT_STAT, flag);
                return Ok(());
            }

            // Check for any error flags
            if int_status & EMMC_INT_ERROR_MASK != 0 {
                // Clear error flags
                self.write_reg16(
                    EMMC_NORMAL_INT_STAT,
                    (int_status & EMMC_INT_ERROR_MASK) as u16,
                );

                // Reset the data circuit with proper error handling
                if let Err(e) = self.reset(EMMC_RESET_DATA) {
                    warn!("Failed to reset data circuit: {:?}", e);
                }

                return Err(SdhciError::DataError);
            }

            delay_us(1000); // Wait for 1 ms before checking again
        }

        // Timeout, return error
        Err(SdhciError::Timeout)
    }
}
