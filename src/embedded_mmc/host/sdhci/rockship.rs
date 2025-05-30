use core::fmt::Display;
use log::{debug, info, trace};

use crate::{delay_us, embedded_mmc::{aux::generic_fls, commands::{DataBuffer, MmcCommand, MmcResponse}, host::{constants::*, sdhci::SdhciError}}, impl_register_ops};

use super::SdhciResult;

#[allow(dead_code)]
const EMMC_DEFAULT_BOUNDARY_ARG: u16 = 7;

const CMD_DEFAULT_TIMEOUT: u32 = 100;
const CMD_MAX_TIMEOUT: u32 = 500;

// SD Host Controller structure
#[derive(Debug)]
pub struct SdhciHost {
    pub base_addr: usize,
    pub caps: u32,
    pub clock_base: u32,
    pub voltages: u32,
    pub quirks: u32,
    pub host_caps: u32,
    pub version: u16,

    pub timing: u32,
    pub bus_width: u8,
    pub clock: u32,
}

impl Display for SdhciHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "EMMC Controller {{ base_addr: {:#x}, caps: {:#x}, clock_base: {} }}",
            self.base_addr, self.caps, self.clock_base
        )
    }
}

impl_register_ops!(SdhciHost, base_addr);

impl SdhciHost {
    // Initialize the host controller
    pub fn init_host(&mut self) -> SdhciResult {
        info!("Init EMMC Controller");

        // Reset the controller
        self.reset(EMMC_RESET_ALL)?;

        let version = self.read_reg16(EMMC_HOST_CNTRL_VER);
        // version = 4.2
        self.version = version;
        info!("EMMC Version: 0x{:x}", version);

        let caps1 = self.read_reg32(EMMC_HOST_CNTRL_VER);
        info!("EMMC Capabilities 1: 0b{:b}", caps1);

        let mut clk_mul: u32 = 0;

        if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 {
            let caps2 = self.read_reg32(EMMC_CAPABILITIES2);
            info!("EMMC Capabilities 2: 0b{:b}", caps2);
            clk_mul = (caps2 & EMMC_CLOCK_MUL_MASK) >> EMMC_CLOCK_MUL_SHIFT;
        }

        if self.clock_base == 0 {
            if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 {
                self.clock_base = (caps1 & EMMC_CLOCK_V3_BASE_MASK) >> EMMC_CLOCK_BASE_SHIFT
            } else {
                self.clock_base = (caps1 & EMMC_CLOCK_BASE_MASK) >> EMMC_CLOCK_BASE_SHIFT
            }

            self.clock_base *= 1000000; // convert to Hz
            if clk_mul != 0 {
                self.clock_base *= clk_mul;
            }
        }

        if self.clock_base == 0 {
            info!("Hardware doesn't specify base clock frequency");
            return Err(SdhciError::UnsupportedCard);
        }

        self.host_caps = MMC_MODE_HS | MMC_MODE_HS_52MHZ | MMC_MODE_4BIT;

        if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 && (caps1 & EMMC_CAN_DO_8BIT) == 0 {
            self.host_caps &= !MMC_MODE_8BIT;
        }

        // 暂时写死
        self.host_caps |= 0x48;

        // debug!("self.host_caps {:#x}", self.host_caps);

        let mut voltages = 0;

        if (caps1 & EMMC_CAN_VDD_330) != 0 {
            voltages |= MMC_VDD_32_33 | MMC_VDD_33_34;
        } else if (caps1 & EMMC_CAN_VDD_300) != 0 {
            voltages |= MMC_VDD_29_30 | MMC_VDD_30_31;
        } else if (caps1 & EMMC_CAN_VDD_180) != 0 {
            voltages |= MMC_VDD_165_195;
        } else {
            info!("Unsupported voltage range");
            return Err(SdhciError::UnsupportedCard);
        }

        self.voltages = voltages;

        info!(
            "voltage range: {:#x}, {:#x}",
            voltages,
            generic_fls(voltages) - 1
        );

        // Perform full power cycle
        self.sdhci_set_power(generic_fls(voltages) - 1).unwrap();

        // Enable interrupts
        self.write_reg32(
            EMMC_NORMAL_INT_STAT_EN,
            EMMC_INT_CMD_MASK | EMMC_INT_DATA_MASK,
        );
        self.write_reg32(EMMC_SIGNAL_ENABLE, 0x0);

        // Set initial bus width to 1-bit
        self.mmc_set_bus_width(1);

        // Set initial clock and wait for it to stabilize
        self.mmc_set_clock(400000);

        self.mmc_set_timing(MMC_TIMING_LEGACY);

        info!("EMMC initialization completed successfully");
        Ok(())
    }

    // Reset the controller
    pub fn reset(&self, mask: u8) -> SdhciResult {
        // Request reset
        self.write_reg8(EMMC_SOFTWARE_RESET, mask);

        // Wait for reset to complete with timeout
        let mut timeout = 20; // Increased timeout
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & mask) != 0 {
            if timeout == 0 {
                return Err(SdhciError::Timeout);
            }
            timeout -= 1;
            delay_us(1000);
        }

        Ok(())
    }

    pub fn mmc_set_bus_width(&mut self, width: u8) {
        /* Set bus width */
        self.bus_width = width;
        debug!("Bus width set to {}", width);
        self.sdhci_set_ios();
    }

    pub fn mmc_set_timing(&mut self, timing: u32) {
        /* Set timing */
        self.timing = timing;
        self.sdhci_set_ios();
    }

    pub fn mmc_set_clock(&mut self, clk: u32) {
        /* Set clock */
        self.clock = clk;
        self.sdhci_set_ios();
    }

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

            {
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
                "EMMC Normal Int Status: 0x{:x}",
                self.read_reg16(EMMC_NORMAL_INT_STAT)
            );
            trace!(
                "EMMC Error Int Status: 0x{:x}",
                self.read_reg16(EMMC_ERROR_INT_STAT)
            );

            let err_status = self.read_reg16(EMMC_ERROR_INT_STAT);
            info!(
                "Command error: status={:#b}, err_status={:#b}",
                status, err_status
            );

            // Reset command and data lines
            self.reset_cmd()?;
            if cmd.data_present {
                self.reset_data()?;
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
                #[cfg(feature = "dma")]
                self.transfer_data_by_dma()?;

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

        self.reset(EMMC_RESET_CMD)?;
        self.reset(EMMC_RESET_DATA)?;

        Ok(())
    }

    // Reset command line
    pub fn reset_cmd(&self) -> SdhciResult {
        self.write_reg8(EMMC_SOFTWARE_RESET, EMMC_RESET_CMD);

        // Wait for reset to complete
        let mut timeout = 100;
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & EMMC_RESET_CMD) != 0 {
            if timeout == 0 {
                return Err(SdhciError::Timeout);
            }
            timeout -= 1;
            delay_us(1000);
        }

        Ok(())
    }

    // Reset data line
    pub fn reset_data(&self) -> SdhciResult {
        self.write_reg8(EMMC_SOFTWARE_RESET, EMMC_RESET_DATA);

        // Wait for reset to complete
        let mut timeout = 100;
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & EMMC_RESET_DATA) != 0 {
            if timeout == 0 {
                return Err(SdhciError::Timeout);
            }
            timeout -= 1;
            delay_us(1000);
        }

        Ok(())
    }

    // Get response from the last command
    pub fn get_response(&self) -> MmcResponse {
        let mut response = MmcResponse::new();
        response.raw[0] = self.read_reg32(EMMC_RESPONSE);
        response.raw[1] = self.read_reg32(EMMC_RESPONSE + 4);
        response.raw[2] = self.read_reg32(EMMC_RESPONSE + 8);
        response.raw[3] = self.read_reg32(EMMC_RESPONSE + 12);

        response
    }

    /// Transfer data using PIO (Programmed I/O) mode
    /// This function manually reads/writes data to/from the controller buffer
    /// Parameters:
    /// - data_dir_read: True for read operation, false for write
    /// - buffer: Buffer to read data into or write data from
    pub fn transfer_data_by_pio(
        &self,
        data_dir_read: bool,
        buffer: &mut [u8],
    ) -> SdhciResult {
        // Process data in 16-byte chunks (4 words at a time)
        for i in (0..buffer.len()).step_by(16) {
            if data_dir_read {
                // Read operation: controller buffer -> memory
                let mut values = [0u32; 4];
                for j in 0..4 {
                    if i + j * 4 < buffer.len() {
                        // Read 32-bit word from controller buffer
                        values[j] = self.read_reg32(EMMC_BUF_DATA);

                        if i + j * 4 + 3 < buffer.len() {
                            // Unpack 32-bit word into 4 bytes in little-endian order
                            buffer[i + j * 4] = (values[j] & 0xFF) as u8;
                            buffer[i + j * 4 + 1] = ((values[j] >> 8) & 0xFF) as u8;
                            buffer[i + j * 4 + 2] = ((values[j] >> 16) & 0xFF) as u8;
                            buffer[i + j * 4 + 3] = ((values[j] >> 24) & 0xFF) as u8;
                        }
                    }
                }

                trace!(
                    "0x{:08x}: 0x{:08x} 0x{:08x} 0x{:08x} 0x{:08x}",
                    buffer.as_ptr() as usize + i,
                    values[0],
                    values[1],
                    values[2],
                    values[3]
                );
            } else {
                // Write operation: memory -> controller buffer
                let mut values = [0u32; 4];
                for j in 0..4 {
                    if i + j * 4 + 3 < buffer.len() {
                        // Pack 4 bytes into 32-bit word in little-endian order
                        values[j] = (buffer[i + j * 4] as u32)
                            | ((buffer[i + j * 4 + 1] as u32) << 8)
                            | ((buffer[i + j * 4 + 2] as u32) << 16)
                            | ((buffer[i + j * 4 + 3] as u32) << 24);

                        // Write 32-bit word to controller buffer
                        self.write_reg32(EMMC_BUF_DATA, values[j]);
                    }
                }

                trace!(
                    "0x{:08x}: 0x{:08x} 0x{:08x} 0x{:08x} 0x{:08x}",
                    buffer.as_ptr() as usize + i,
                    values[0],
                    values[1],
                    values[2],
                    values[3]
                );
            }
        }

        Ok(())
    }

    /// Write data to SD card buffer register
    /// This is a lower-level function used by data transfer operations
    pub fn write_buffer(&self, buffer: &[u8]) -> SdhciResult {
        // Wait until space is available in the controller buffer
        self.wait_for_interrupt(EMMC_INT_SPACE_AVAIL, 100000)?;

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

            // Write the 32-bit word to the buffer data register
            self.write_reg32(EMMC_BUF_DATA, val);
        }

        // Wait for data transfer to complete
        self.wait_for_interrupt(EMMC_INT_DATA_END, 1000000)?;

        Ok(())
    }

    /// Read data from SD card buffer register
    /// This is a lower-level function used by data transfer operations
    pub fn read_buffer(&self, buffer: &mut [u8]) -> SdhciResult {
        // Wait until data is available in the controller buffer
        self.wait_for_interrupt(EMMC_INT_DATA_AVAIL, 100000)?;

        // Read data into buffer in 4-byte chunks
        let len = buffer.len();
        for i in (0..len).step_by(4) {
            // Read 32-bit word from buffer data register
            let val = self.read_reg32(EMMC_BUF_DATA);

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
        self.wait_for_interrupt(EMMC_INT_DATA_END, 100000)?;

        Ok(())
    }

    /// Wait for a specific interrupt flag to be set
    /// Helper function used by data transfer operations
    /// Parameters:
    /// - flag: The interrupt flag to wait for
    /// - timeout_count: Maximum number of iterations to wait
    fn wait_for_interrupt(&self, flag: u32, timeout_count: u32) -> SdhciResult {
        let mut timeout = timeout_count;
        while timeout > 0 {
            // Read the current interrupt status
            let int_status = self.read_reg32(EMMC_NORMAL_INT_STAT);

            // Check if the target flag is set
            if int_status & flag != 0 {
                // Clear the flag by writing back to the register
                self.write_reg16(EMMC_NORMAL_INT_STAT, flag as u16);
                return Ok(());
            }

            // Check for any error flags
            if int_status & EMMC_INT_ERROR_MASK != 0 {
                // Clear error flags
                self.write_reg16(
                    EMMC_NORMAL_INT_STAT,
                    (int_status & EMMC_INT_ERROR_MASK) as u16,
                );
                // Reset the data circuit
                self.reset_data().unwrap();
                return Err(SdhciError::DataError);
            }

            timeout -= 1;
        }

        // If we reached the timeout limit, return timeout error
        if timeout == 0 {
            return Err(SdhciError::Timeout);
        }

        Ok(())
    }
}