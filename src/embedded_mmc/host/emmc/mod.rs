mod block;
mod cmd;
mod err;
mod ext;

use core::fmt::Display;
use err::SdError;
use log::{debug, info};

use crate::{delay_us, embedded_mmc::{aux::generic_fls, commands::MmcCommand, host::constants::*}, emmc::aux::{lldiv, MMC_VERSION_4}, impl_register_ops};

// SD Host Controller structure
#[derive(Debug)]
pub struct EMmcHost {
    base_addr: usize,
    caps: u32,
    clock_base: u32,
    voltages: u32,
    quirks: u32,
    host_caps: u32,
    version: u16,

    timing: u32,
    bus_width: u8,
    clock: u32,
}

impl Display for EMmcHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "EMMC Controller {{ base_addr: {:#x}, caps: {:#x}, clock_base: {} }}",
            self.base_addr, self.caps, self.clock_base
        )
    }
}

impl_register_ops!(EMmcHost, base_addr);

impl EMmcHost {
    // Initialize the host controller
    pub fn init_host(&mut self) -> Result<(), SdError> {
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
            return Err(SdError::UnsupportedCard);
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
            return Err(SdError::UnsupportedCard);
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
    pub fn reset(&self, mask: u8) -> Result<(), SdError> {
        // Request reset
        self.write_reg8(EMMC_SOFTWARE_RESET, mask);

        // Wait for reset to complete with timeout
        let mut timeout = 20; // Increased timeout
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & mask) != 0 {
            if timeout == 0 {
                return Err(SdError::Timeout);
            }
            timeout -= 1;
            delay_us(1000);
        }

        Ok(())
    }

    fn mmc_set_capacity(&mut self, part_num: u32) -> Result<(), SdError> {
        // part_num 暂时设置为 0
        match part_num {
            0 => match self.capacity_user() {
                Some(capacity_user) => self.set_capacity(capacity_user).unwrap(),
                None => return Err(SdError::InvalidArgument),
            },
            1 | 2 => match self.capacity_boot() {
                Some(capacity_boot) => self.set_capacity(capacity_boot).unwrap(),
                None => return Err(SdError::InvalidArgument),
            },
            3 => match self.capacity_rpmb() {
                Some(capacity_rpmb) => self.set_capacity(capacity_rpmb).unwrap(),
                None => return Err(SdError::InvalidArgument),
            },
            4..=7 => match self.capacity_gp() {
                Some(capacity_gp) => self
                    .set_capacity(capacity_gp[(part_num - 4) as usize])
                    .unwrap(),
                None => return Err(SdError::InvalidArgument),
            },
            _ => return Err(SdError::InvalidArgument),
        }

        let capacity = self.capacity().unwrap_or(0);
        let read_bl_len = self.read_bl_len().unwrap_or(0);
        let _lba = lldiv(capacity, read_bl_len);

        Ok(())
    }

    pub fn mmc_change_freq(&mut self) -> Result<(), SdError> {
        // Allocate buffer for EXT_CSD depending on whether DMA or PIO is enabled
        cfg_if::cfg_if! {
            if #[cfg(feature = "dma")] {
                let mut ext_csd: DVec<u8> = DVec::zeros(MMC_MAX_BLOCK_LEN as usize, 0x1000, Direction::FromDevice).unwrap();
            } else if #[cfg(feature = "pio")] {
                let mut ext_csd: [u8; 512] = [0; 512];
            }
        }

        // Initialize card capabilities flags
        self.set_card_caps(0).unwrap();

        // Get card version (default to 0 if not available)
        let version = self.version().unwrap_or(0);

        // Only cards version 4.0 and above support high-speed modes
        if version < MMC_VERSION_4 {
            return Ok(());
        }

        // Enable both 4-bit and 8-bit modes on the card
        self.set_card_caps(MMC_MODE_4BIT | MMC_MODE_8BIT).unwrap();

        // Read the EXT_CSD register from the card
        self.mmc_send_ext_csd(&mut ext_csd)?;

        // Determine supported high-speed modes from EXT_CSD
        let avail_type = self.mmc_select_card_type(&ext_csd);

        // Select the appropriate high-speed mode supported by both host and card
        let result = if avail_type & EXT_CSD_CARD_TYPE_HS200 != 0 {
            // HS200 mode
            self.mmc_select_hs200()
        } else if avail_type & EXT_CSD_CARD_TYPE_HS != 0 {
            // Standard high-speed mode
            self.mmc_select_hs()
        } else {
            Err(SdError::InvalidArgument)
        };

        // Apply the result of speed mode selection
        result?;

        // Configure the bus speed according to selected type
        self.mmc_set_bus_speed(avail_type as u32);

        // If HS200 mode was selected, perform tuning procedure
        if self.mmc_card_hs200() {
            let tuning_result = self.mmc_hs200_tuning();

            // Optionally upgrade to HS400 mode if supported and using 8-bit bus
            if avail_type & EXT_CSD_CARD_TYPE_HS400 != 0
                && self.bus_width == MMC_BUS_WIDTH_8BIT
            {
                // self.mmc_select_hs400()?; // Currently not executed
                self.mmc_set_bus_speed(avail_type as u32);
            }

            tuning_result
        } else if !self.mmc_card_hs400es() {
            // If not in HS400 Enhanced Strobe mode, try to switch bus width
            let width_result = self.mmc_select_bus_width()?;
            let err = if width_result > 0 {
                Ok(())
            } else {
                Err(SdError::BusWidth)
            };

            // If DDR52 mode is supported, implement selection (currently TODO)
            if err.is_ok() && avail_type & EXT_CSD_CARD_TYPE_DDR_52 as u16 != 0 {
                todo!("Implement HS-DDR selection");
            }

            err
        } else {
            // Already in HS400ES mode, no further action needed
            Ok(())
        }
    }

    pub fn mmc_set_bus_speed(&mut self, avail_type: u32) {
        let mut clock = 0;

        if self.mmc_card_hs() {
            clock = if (avail_type & EXT_CSD_CARD_TYPE_52 as u32) != 0 {
                MMC_HIGH_52_MAX_DTR
            } else {
                MMC_HIGH_26_MAX_DTR
            };
        } else if self.mmc_card_hs200() {
            clock = MMC_HS200_MAX_DTR;
        }

        self.mmc_set_clock(clock);
    }

    /// 检查卡是否为HS模式
    fn mmc_card_hs(&self) -> bool {
        let timing = self.timing;
        (timing == MMC_TIMING_MMC_HS) || (timing == MMC_TIMING_SD_HS)
    }

    fn mmc_card_hs400es(&self) -> bool {
        let timing = self.timing;
        timing == MMC_TIMING_MMC_HS400ES
    }

    /// 检查卡是否为HS200模式
    fn mmc_card_hs200(&self) -> bool {
        let timing = self.timing;
        timing == MMC_TIMING_MMC_HS200
    }

    pub fn mmc_select_hs200(&mut self) -> Result<(), SdError> {
        let ret = self.mmc_select_bus_width()?;

        if ret > 0 {
            self.mmc_switch(
                EXT_CSD_CMD_SET_NORMAL,
                EXT_CSD_HS_TIMING,
                EXT_CSD_TIMING_HS200,
                false,
            )?;

            self.mmc_set_timing(MMC_TIMING_MMC_HS200);
        }

        Ok(())
    }

    fn mmc_select_bus_width(&mut self) -> Result<i32, SdError> {
        let ext_csd_bits: [u8; 2] = [EXT_CSD_BUS_WIDTH_8, EXT_CSD_BUS_WIDTH_4];
        let bus_widths: [u8; 2] = [MMC_BUS_WIDTH_8BIT, MMC_BUS_WIDTH_4BIT];

        cfg_if::cfg_if! {
            if #[cfg(feature = "dma")] {
                let mut ext_csd: DVec<u8> = DVec::zeros(MMC_MAX_BLOCK_LEN as usize, 0x1000, Direction::FromDevice).unwrap();
                let mut test_csd = DVec::zeros(MMC_MAX_BLOCK_LEN as usize, 0x1000, Direction::FromDevice)
        .ok_or(SdError::MemoryError)?;
            } else if #[cfg(feature = "pio")] {
                let mut ext_csd: [u8; 512] = [0; 512];
                let mut test_csd: [u8; 512] = [0; 512];
            }
        }

        // 版本检查和主机能力检查
        if self.version().unwrap_or(0) < MMC_VERSION_4
            || (self.host_caps & (MMC_MODE_4BIT | MMC_MODE_8BIT)) == 0
        {
            return Ok(0);
        }

        self.mmc_send_ext_csd(&mut ext_csd)?;

        let mut idx = if (self.host_caps & MMC_MODE_8BIT) != 0 {
            0
        } else {
            1
        };
        while idx < bus_widths.len() {
            let switch_result = self.mmc_switch(
                EXT_CSD_CMD_SET_NORMAL,
                EXT_CSD_BUS_WIDTH,
                ext_csd_bits[idx],
                true,
            );

            if switch_result.is_err() {
                idx += 1;
                continue;
            }

            let bus_width = bus_widths[idx];
            self.mmc_set_bus_width(bus_width);

            // 再次读取EXT_CSD进行验证
            let test_result = self.mmc_send_ext_csd(&mut test_csd);

            if test_result.is_err() {
                idx += 1;
                continue;
            }
            if (ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize]
                == test_csd[EXT_CSD_PARTITIONING_SUPPORT as usize])
                && (ext_csd[EXT_CSD_HC_WP_GRP_SIZE as usize]
                    == test_csd[EXT_CSD_HC_WP_GRP_SIZE as usize])
                && (ext_csd[EXT_CSD_REV as usize] == test_csd[EXT_CSD_REV as usize])
                && (ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize]
                    == test_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize])
                && self.compare_sector_count(&ext_csd, &test_csd)
            {
                return Ok(bus_width as i32);
            } else {
                idx += 1;
            }
        }

        Err(SdError::BadMessage)
    }

    #[cfg(feature = "pio")]
    fn compare_sector_count(&self, ext_csd: &[u8], test_csd: &[u8]) -> bool {
        let sec_cnt_offset = EXT_CSD_SEC_CNT as usize;
        for i in 0..4 {
            if ext_csd[sec_cnt_offset + i] != test_csd[sec_cnt_offset + i] {
                return false;
            }
        }
        true
    }

    /// Perform HS200 tuning sequence (also used for HS400 initial tuning)
    fn mmc_hs200_tuning(&mut self) -> Result<(), SdError> {
        let opcode = MMC_SEND_TUNING_BLOCK_HS200;
        let timing = self.timing;

        match timing {
            // HS400 tuning must be issued in HS200 mode; reject direct HS400 timing
            MMC_TIMING_MMC_HS400 => {
                return Err(SdError::InvalidArgument);
            }
            // HS200 timing: OK to proceed with tuning here
            MMC_TIMING_MMC_HS200 => {
                // HS400 re-tuning is not expected; leave periodic tuning disabled
            }
            // Any other timing mode is invalid for HS200 tuning
            _ => {
                return Err(SdError::InvalidArgument);
            }
        }

        // Set the EXEC_TUNING bit in Host Control2 to start tuning
        let mut ctrl = self.read_reg16(EMMC_HOST_CTRL2);
        ctrl |= MMC_CTRL_EXEC_TUNING;
        self.write_reg16(EMMC_HOST_CTRL2, ctrl);

        // Invoke the common tuning loop implementation
        self.__emmc_execute_tuning(opcode)
    }

    /// Core tuning loop: send tuning blocks until the controller indicates success or timeout
    fn __emmc_execute_tuning(&mut self, opcode: u8) -> Result<(), SdError> {
        const MAX_TUNING_LOOP: usize = 40;

        for _ in 0..MAX_TUNING_LOOP {
            // Send one tuning block command
            self.emmc_send_tuning(opcode)?;

            // Read back Host Control2 to check tuning status
            let ctrl = self.read_reg16(EMMC_HOST_CTRL2);

            // If the EXEC_TUNING bit has been cleared by hardware...
            if (ctrl & MMC_CTRL_EXEC_TUNING) == 0 {
                // ...and the TUNED_CLK bit is set, tuning succeeded
                if (ctrl & MMC_CTRL_TUNED_CLK) != 0 {
                    return Ok(());
                }
                // EXEC_TUNING cleared but no TUNED_CLK => break and report failure
                break;
            }
        }

        // Exceeded max loops without success: timeout
        Err(SdError::Timeout)
    }

    /// Send a single tuning block read command over the SDHCI interface
    fn emmc_send_tuning(&mut self, opcode: u8) -> Result<(), SdError> {
        // Helper to pack DMA boundary and block size fields
        let make_blksz = |dma: u16, blksz: u16| ((dma & 0x7) << 12) | (blksz & 0x0FFF);

        // Determine current bus width (1/4/8 bits)
        let bus_width = self.bus_width;

        // Choose block size: 128 bytes for HS200 on 8-bit bus, else 64 bytes
        let block_size = if opcode == MMC_SEND_TUNING_BLOCK_HS200 && bus_width == MMC_BUS_WIDTH_8BIT
        {
            128
        } else {
            64
        };

        // Program block size and enable DMA boundary
        self.write_reg16(EMMC_BLOCK_SIZE, make_blksz(7, block_size));
        // Set transfer mode to single-block read
        self.write_reg16(EMMC_XFER_MODE, EMMC_TRNS_READ);

        // Build and send the tuning command
        let cmd = MmcCommand::new(opcode, 0, MMC_RSP_R1);
        self.send_command(&cmd, None)?;

        Ok(())
    }

    #[allow(unused)]
    fn mmc_card_ddr(&self) -> bool {
        let timing = self.timing;
        (timing == MMC_TIMING_UHS_DDR50)
            || (timing == MMC_TIMING_MMC_DDR52)
            || (timing == MMC_TIMING_MMC_HS400)
            || (timing == MMC_TIMING_MMC_HS400ES)
    }

    pub fn mmc_select_card_type(&self, ext_csd: &[u8]) -> u16 {
        let card_type = ext_csd[EXT_CSD_CARD_TYPE as usize] as u16;
        let host_caps = self.host_caps;
        let mut avail_type = 0;

        if (host_caps & MMC_MODE_HS != 0) && (card_type & EXT_CSD_CARD_TYPE_26 != 0) {
            avail_type |= EXT_CSD_CARD_TYPE_26;
        }

        if (host_caps & MMC_MODE_HS != 0) && (card_type & EXT_CSD_CARD_TYPE_52 != 0) {
            avail_type |= EXT_CSD_CARD_TYPE_52;
        }

        if (host_caps & MMC_MODE_DDR_52MHZ != 0)
            && (card_type & EXT_CSD_CARD_TYPE_DDR_1_8V as u16 != 0)
        {
            avail_type |= EXT_CSD_CARD_TYPE_DDR_1_8V as u16;
        }

        if (host_caps & MMC_MODE_HS200 != 0) && (card_type & EXT_CSD_CARD_TYPE_HS200_1_8V != 0) {
            avail_type |= EXT_CSD_CARD_TYPE_HS200_1_8V;
        }

        if (host_caps & MMC_MODE_HS400 != 0)
            && (host_caps & MMC_MODE_8BIT != 0)
            && (card_type & EXT_CSD_CARD_TYPE_HS400_1_8V != 0)
        {
            avail_type |= EXT_CSD_CARD_TYPE_HS200_1_8V | EXT_CSD_CARD_TYPE_HS400_1_8V;
        }

        if (host_caps & MMC_MODE_HS400ES != 0)
            && (host_caps & MMC_MODE_8BIT != 0)
            && (ext_csd[EXT_CSD_STROBE_SUPPORT as usize] != 0)
            && (avail_type & EXT_CSD_CARD_TYPE_HS400_1_8V != 0)
        {
            avail_type |= EXT_CSD_CARD_TYPE_HS200_1_8V
                | EXT_CSD_CARD_TYPE_HS400_1_8V
                | EXT_CSD_CARD_TYPE_HS400ES;
        }

        avail_type
    }

    fn mmc_select_hs(&mut self) -> Result<(), SdError> {
        let ret = self.mmc_switch(
            EXT_CSD_CMD_SET_NORMAL,
            EXT_CSD_HS_TIMING,
            EXT_CSD_TIMING_HS,
            true,
        );

        if ret.is_ok() {
            self.mmc_set_timing(MMC_TIMING_MMC_HS);
        }

        ret
    }

    fn mmc_set_bus_width(&mut self, width: u8) {
        /* Set bus width */
        self.bus_width = width;
        debug!("Bus width set to {}", width);
        self.sdhci_set_ios();
    }

    fn mmc_set_timing(&mut self, timing: u32) {
        /* Set timing */
        self.timing = timing;
        self.sdhci_set_ios();
    }

    fn mmc_set_clock(&mut self, clk: u32) {
        /* Set clock */
        self.clock = clk;
        self.sdhci_set_ios();
    }

    fn mmc_switch(
        &self,
        _set: u8,
        index: u32,
        value: u8,
        send_status: bool,
    ) -> Result<(), SdError> {
        let mut retries = 3;
        let cmd = MmcCommand::new(
            MMC_SWITCH,
            (MMC_SWITCH_MODE_WRITE_BYTE << 24)
                | (index << 16)
                | ((value as u32) << 8),
            MMC_RSP_R1B,
        );

        loop {
            let ret = self.send_command(&cmd, None);

            if ret.is_ok() {
                debug!("cmd6 {:#x}", self.get_response().as_r1());
                return self.mmc_poll_for_busy(send_status);
            }

            retries -= 1;
            if retries <= 0 {
                debug!("Switch command failed after 3 retries");
                break;
            }
        }

        Err(SdError::Timeout)
    }
}