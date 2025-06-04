use crate::{
    commands::{DataBuffer, MmcCommand},
    constants::*,
    delay_us,
    host::rockship::{SdhciError, SdhciHost, SdhciResult},
};
use log::{info, trace};

const CMD_DEFAULT_TIMEOUT: u32 = 100;
const CMD_MAX_TIMEOUT: u32 = 500;
// const READ_STATUS_TIMEOUT: u32 = 1000;

impl SdhciHost {
    // Send command
    pub fn send_command(
        &self,
        cmd: &MmcCommand,
        mut data_buffer: Option<DataBuffer>,
    ) -> SdhciResult {
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
                _ => return Err(SdhciError::InvalidValue),
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
                return Err(SdhciError::Timeout);
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
                SdhciError::Timeout
            } else {
                SdhciError::CommandError
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
                return Err(SdhciError::InvalidValue);
            }
        }

        // Clear all interrupt statuses
        self.write_reg16(EMMC_NORMAL_INT_STAT, 0xFFFF);
        self.write_reg16(EMMC_ERROR_INT_STAT, 0xFFFF);

        Ok(())
    }
}
