use log::{debug, info, warn};

use super::{SdHost, constant::*};
use crate::delay_us;
use crate::err::SdError;

#[derive(Debug)]
pub struct SdCommand {
    pub opcode: u8,
    pub arg: u32,
    pub resp_type: u32,
    pub data_present: bool,
    pub data_dir_read: bool,
    pub block_size: u16,
    pub block_count: u16,
}

impl SdCommand {
    pub fn new(opcode: u8, arg: u32, resp_type: u32) -> Self {
        Self {
            opcode,
            arg,
            resp_type,
            data_present: false,
            data_dir_read: true,
            block_size: 0,
            block_count: 0,
        }
    }

    pub fn with_data(mut self, block_size: u16, block_count: u16, is_read: bool) -> Self {
        self.data_present = true;
        self.data_dir_read = is_read;
        self.block_size = block_size;
        self.block_count = block_count;
        self
    }
}

pub struct SdResponse {
    pub raw: [u32; 4],
}

impl SdResponse {
    pub fn new() -> Self {
        Self { raw: [0; 4] }
    }

    pub fn as_r1(&self) -> u32 {
        self.raw[0]
    }

    pub fn as_r2(&self) -> [u32; 4] {
        self.raw
    }

    pub fn as_r3(&self) -> u32 {
        self.raw[0]
    }

    pub fn as_r6(&self) -> u32 {
        self.raw[0]
    }

    pub fn as_r7(&self) -> u32 {
        self.raw[0]
    }
}

impl SdHost {
    // Send a command to the card
    pub fn send_command(&self, cmd: &SdCommand) -> Result<(), SdError> {
        // Check if command or data lines are busy
        let mut timeout = 100000;
        let mut time: u32 = 0;
        let mut mask: u32 = 0;
        let mut flags: u16;
        let mut mode: u16;
        let mut ret: i32 = 0;

        if cmd.data_present {
            mask |= SDHCI_DATA_INHIBIT;
        }

        if cmd.opcode == MMC_STOP_TRANSMISSION {
            mask &= !SDHCI_DATA_INHIBIT;
        }

        // 循环直到状态寄存器的值不再符合掩码条件，或者超时
        while self.read_reg(SDHCI_PRESENT_STATE) & mask != 0 {
            if time >= timeout {
                if 2 * timeout <= 3200 {
                    timeout *= 2; // 将超时时间翻倍
                    debug!("timeout increasing to: {} ms.", timeout);
                    self.write_reg(SDHCI_INT_STATUS, SDHCI_INT_ALL_MASK); // 清除中断状态
                } else {
                    debug!("timeout.");
                    // 超过最大超时，退出循环
                    break;
                }
            }
            time += 1;
            delay_us(1000);
        }

        // Clear interrupt status
        self.write_reg(SDHCI_INT_STATUS, SDHCI_INT_ALL_MASK);
        mask = SDHCI_INT_RESPONSE;

        if cmd.resp_type & MMC_RSP_PRESENT == 0 {
            flags = SDHCI_CMD_RESP_NONE;
        } else if cmd.resp_type & MMC_RSP_136 != 0 {
            flags = SDHCI_CMD_RESP_LONG;
        } else if cmd.resp_type & MMC_RSP_BUSY != 0 {
            flags = SDHCI_CMD_RESP_SHORT_BUSY;
            if cmd.data_present {
                mask |= SDHCI_INT_DATA_END;
            }
        } else {
            flags = SDHCI_CMD_RESP_SHORT;
        }

        if cmd.resp_type & MMC_RSP_CRC != 0 {
            flags |= SDHCI_CMD_CRC;
        }
        if cmd.resp_type & MMC_RSP_OPCODE != 0 {
            flags |= SDHCI_CMD_INDEX;
        }
        if cmd.arg != 0 {
            flags |= SDHCI_CMD_DATA;
        }

        if cmd.opcode == MMC_SEND_TUNING_BLOCK || cmd.opcode == MMC_SEND_TUNING_BLOCK_HS200 {
            mask &= !SDHCI_INT_RESPONSE;
            mask |= SDHCI_INT_DATA_AVAIL;
            flags |= SDHCI_CMD_DATA;
        }

        debug!("checkpoint 05");
        if cmd.data_present{
            //self.write_reg(SDHCI_TIMEOUT_CONTROL, 0xe); // 假设写操作的功能
            mode = SDHCI_TRNS_BLK_CNT_EN;
            if cmd.block_count > 1 {
                mode |= SDHCI_TRNS_MULTI;
            }

             if cmd.opcode as u16 == SDHCI_CMD_DATA {
                 mode |= SDHCI_TRNS_READ;
             }

            self.write_reg(
                SDHCI_BLOCK_SIZE,
                sdhci_make_blksz(SDHCI_DEFAULT_BOUNDARY_ARG, cmd.block_size as u32),
            );

            self.write_reg(SDHCI_BLOCK_COUNT, cmd.block_count as u32);
            self.write_reg(SDHCI_TRANSFER_MODE, mode as u32);
        } else if cmd.resp_type == MMC_RSP_BUSY {
            self.write_reg(SDHCI_TIMEOUT_CONTROL, 0xe);
        }

        self.write_reg(SDHCI_ARGUMENT, cmd.arg);
        self.write_reg16(
            SDHCI_COMMAND,
            sdhci_make_cmd(cmd.opcode as u16, flags),
        );
        debug!("checkpoint 07");

        let start = get_timer(0);
        let mut stat = self.read_reg(SDHCI_INT_STATUS);
        loop {
            stat = self.read_reg(SDHCI_INT_STATUS);
            if stat & SDHCI_INT_ERROR != 0 {
                break;
            }

            if get_timer(start) >= 1000 {
                if  SDHCI_QUIRK_BROKEN_R1B != 0 {       //incomplete
                    return Ok(());
                } else {
                    debug!("{}: Timeout for status update!", "update_status");
                    return Err(SdError::Timeout);
                }
            }

            if stat & mask == mask {
                break;
            }
        }
        if (stat & (SDHCI_INT_ERROR | mask)) == mask {
            //sdhci_cmd_done(host, cmd);
            self.write_reg( SDHCI_INT_STATUS, mask);
        } else {
            ret = -1;
        }

        delay_us(1000);
        stat = self.read_reg( SDHCI_INT_STATUS);
	    self.write_reg(SDHCI_INT_STATUS, SDHCI_INT_ALL_MASK);;
        if ret != 0 {
            debug!("cmd: error: {}.", ret);
            return Ok(()); // 在 Rust 中返回 0
        }
        let _= self.reset_cmd();
        let _= self.reset_data();

        if stat & SDHCI_INT_TIMEOUT != 0 {
            return Err(SdError::Timeout);
        } else {
		    return Ok(());
        }
        return Ok(());
    }


    // Get response from the last command
    pub fn get_response(&self) -> SdResponse {
        let mut response = SdResponse::new();
        response.raw[0] = self.read_reg(SDHCI_RESPONSE);
        response.raw[1] = self.read_reg(SDHCI_RESPONSE + 4);
        response.raw[2] = self.read_reg(SDHCI_RESPONSE + 8);
        response.raw[3] = self.read_reg(SDHCI_RESPONSE + 12);
        response
    }
}

pub fn sdhci_make_blksz(dma: u32, blksz: u32) -> u32 {
    ((dma & 0x7) << 12) | (blksz & 0xFFF)
}

pub fn sdhci_make_cmd(c: u16, f: u16) -> u16 {
    ((c & 0xff) << 8) | (f & 0xff)
}

pub fn get_timer(start: u32) -> u32 {
    start + 100
}