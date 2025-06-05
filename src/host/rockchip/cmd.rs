use core::sync::atomic::{Ordering, fence};

use crate::{
    common::commands::{DataBuffer, MmcCommand},
    constants::*,
    delay_us,
    host::{MmcHostError, MmcHostResult, rockchip::SdhciHost},
};
use log::{info, trace, warn};

const CMD_DEFAULT_TIMEOUT: u32 = 100;
const CMD_MAX_TIMEOUT: u32 = 500;
// const READ_STATUS_TIMEOUT: u32 = 1000;

impl SdhciHost {
    // Send command
    pub fn send_command(
        &self,
        cmd: &MmcCommand,
        mut data_buffer: Option<DataBuffer>,
    ) -> MmcHostResult {
        let mut cmd_timeout = CMD_DEFAULT_TIMEOUT;

        // Check if command or data line is busy
        let mut mask = EMMC_CMD_INHIBIT;
        if cmd.data_present {
            mask |= EMMC_DATA_INHIBIT;
        }

        // For STOP_TRANSMISSION command, no need to wait for data inhibit
        if cmd.opcode == MMC_STOP_TRANSMISSION {
            mask &= !EMMC_DATA_INHIBIT;
        }

        // Wait using dynamically adjusted timeout
        let mut time: u32 = 0;
        while (self.read_reg32(EMMC_PRESENT_STATE) & mask) != 0 {
            if time >= cmd_timeout {
                info!("MMC: busy timeout");

                // If timeout can be increased, double the timeout and continue
                if 2 * cmd_timeout <= CMD_MAX_TIMEOUT {
                    cmd_timeout += cmd_timeout;
                    info!("timeout increasing to: {} ms.", cmd_timeout);
                    self.write_reg16(EMMC_NORMAL_INT_STAT, 0xFFFF);
                } else {
                    info!("timeout.");
                    // Do not return an error, attempt to continue sending the command
                    break;
                }
            }
            time += 1;
            delay_us(1000);
        }

        // Clear all interrupt statuses
        self.write_reg16(EMMC_NORMAL_INT_STAT, 0xFFFF);
        self.write_reg16(EMMC_ERROR_INT_STAT, 0xFFFF);

        let mut int_mask = EMMC_INT_RESPONSE as u16;

        // If data is present and the response type includes the BUSY flag, wait for data end interrupt
        if cmd.data_present && (cmd.resp_type & MMC_RSP_BUSY != 0) {
            int_mask |= EMMC_INT_DATA_END as u16;
        }

        // Set data transfer-related registers
        if cmd.data_present {
            self.write_reg8(EMMC_TIMEOUT_CONTROL, 0xe);

            let mut mode = EMMC_TRNS_BLK_CNT_EN;

            if cmd.block_count > 1 {
                mode |= EMMC_TRNS_MULTI;
            }

            if cmd.data_dir_read {
                mode |= EMMC_TRNS_READ;
            }

            self.write_reg16(
                EMMC_BLOCK_SIZE,
                (cmd.block_size & 0xFFF).try_into().unwrap(),
            );
            self.write_reg16(EMMC_BLOCK_COUNT, cmd.block_count);

            self.write_reg16(EMMC_XFER_MODE, mode);
            match data_buffer {
                Some(DataBuffer::Read(_)) if cmd.data_dir_read => {}
                Some(DataBuffer::Write(_)) if !cmd.data_dir_read => {}
                _ => return Err(MmcHostError::InvalidValue),
            }
        } else if cmd.resp_type & MMC_RSP_BUSY != 0 {
            // For commands with BUSY but no data, still set timeout control
            self.write_reg8(EMMC_TIMEOUT_CONTROL, 0xe);
        }

        // Set parameters
        self.write_reg32(EMMC_ARGUMENT, cmd.arg);

        // Set command register
        let mut command = (cmd.opcode as u16) << 8;

        if cmd.opcode == MMC_SEND_TUNING_BLOCK || cmd.opcode == MMC_SEND_TUNING_BLOCK_HS200 {
            int_mask &= !EMMC_INT_RESPONSE as u16;
            int_mask |= EMMC_INT_DATA_AVAIL as u16;
            command |= EMMC_CMD_DATA;
        }

        // Map response type
        if cmd.resp_type & MMC_RSP_PRESENT != 0 {
            if cmd.resp_type & MMC_RSP_136 != 0 {
                command |= EMMC_CMD_RESP_LONG;
            } else if cmd.resp_type & MMC_RSP_BUSY != 0 {
                command |= EMMC_CMD_RESP_SHORT_BUSY;
            } else {
                command |= EMMC_CMD_RESP_SHORT;
            }
        }

        if cmd.resp_type & MMC_RSP_CRC != 0 {
            command |= EMMC_CMD_CRC;
        }

        if cmd.resp_type & MMC_RSP_OPCODE != 0 {
            command |= EMMC_CMD_INDEX;
        }

        if cmd.data_present {
            command |= EMMC_CMD_DATA;
        }

        trace!(
            "Sending command: opcode={:#x}, arg={:#x}, resp_type={:#x}, command={:#x}",
            cmd.opcode, cmd.arg, cmd.resp_type, command
        );

        // Special command handling
        let mut timeout_val = if cmd.opcode == MMC_GO_IDLE_STATE || cmd.opcode == MMC_SEND_OP_COND {
            CMD_MAX_TIMEOUT
        } else {
            CMD_DEFAULT_TIMEOUT
        };

        // Send the command
        self.write_reg16(EMMC_COMMAND, command);

        // Wait for command completion
        let mut status: u16;
        loop {
            status = self.read_reg16(EMMC_NORMAL_INT_STAT);
            trace!("Response Status: {:#b}", status);

            // Check for errors
            if status & EMMC_INT_ERROR as u16 != 0 {
                break;
            }

            // Check for response completion
            if (status & int_mask) == int_mask {
                break;
            }

            // Check for timeout
            if timeout_val == 0 {
                info!("Timeout for status update!");
                return Err(MmcHostError::Timeout);
            }

            timeout_val -= 1;
            delay_us(100);
        }

        // Process command completion
        if (status & (EMMC_INT_ERROR as u16 | int_mask)) == int_mask {
            // Command successfully completed
            trace!("Command completed: status={:#b}", status);
            self.write_reg16(EMMC_NORMAL_INT_STAT, int_mask);
        } else {
            // Error occurred
            trace!(
                "EMMC Normal Int Status: 0x{:x}, EMMC Error Int Status: 0x{:x}",
                self.read_reg16(EMMC_NORMAL_INT_STAT),
                self.read_reg16(EMMC_ERROR_INT_STAT)
            );

            let err_status = self.read_reg16(EMMC_ERROR_INT_STAT);
            info!(
                "Command error: status={:#b}, err_status={:#b}",
                status, err_status
            );

            // Reset command and data lines
            self.reset(EMMC_RESET_CMD)?;
            if cmd.data_present {
                self.reset(EMMC_RESET_DATA)?;
            }

            // Map specific error types
            let err = if err_status & 0x1 != 0 {
                MmcHostError::Timeout
            } else {
                MmcHostError::CommandError
            };

            return Err(err);
        }

        // Process data transfer part
        if cmd.data_present {
            trace!("Data transfer: cmd.data_present={}", cmd.data_present);
            if let Some(buffer) = &mut data_buffer {
                #[cfg(not(feature = "dma"))]
                match buffer {
                    DataBuffer::Read(buf) => self.read_buffer(buf).unwrap(),
                    DataBuffer::Write(buf) => self.write_buffer(buf).unwrap(),
                }
            } else {
                return Err(MmcHostError::InvalidValue);
            }
        }

        // Clear all interrupt statuses
        self.write_reg16(EMMC_NORMAL_INT_STAT, 0xFFFF);
        self.write_reg16(EMMC_ERROR_INT_STAT, 0xFFFF);

        Ok(())
    }

    /// Write data to SD card buffer register
    /// This is a lower-level function used by data transfer operations
    pub fn write_buffer(&self, buffer: &[u8]) -> MmcHostResult {
        // Wait until space is available in the controller buffer
        self.wait_for_interrupt(EMMC_INT_SPACE_AVAIL, 100)?;

        let len = buffer.len();

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
    fn read_buffer(&self, buffer: &mut [u8]) -> MmcHostResult {
        // Wait until data is available in the controller buffer
        self.wait_for_interrupt(EMMC_INT_DATA_AVAIL, 100)?;

        // Read data into buffer in 4-byte chunks
        let len = buffer.len();

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
    fn wait_for_interrupt(&self, flag: u32, timeout_count: u32) -> MmcHostResult {
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

                return Err(MmcHostError::DataError);
            }

            delay_us(1000); // Wait for 1 ms before checking again
        }

        // Timeout, return error
        Err(MmcHostError::Timeout)
    }
}
