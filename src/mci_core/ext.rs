use crate::{
    common::commands::MmcCommand,
    constants::*,
    host::{MmcHostError, MmcHostOps, MmcHostResult},
    mci_core::MmcHost,
};

impl<T: MmcHostOps> MmcHost<T> {
    pub fn mmc_set_bus_width(&mut self, width: u8) {
        /* Set bus width */
        self.mmc_host_info_mut().set_bus_width(width);
        let host_info = self.mmc_host_info().clone();

        self.host_ops_mut().mmc_set_ios(&host_info)
    }

    pub fn mmc_set_timing(&mut self, timing: u32) {
        /* Set timing */
        self.mmc_host_info_mut().set_timing(timing);
        let host_info = self.mmc_host_info().clone();

        self.host_ops_mut().mmc_set_ios(&host_info)
    }

    pub fn mmc_set_clock(&mut self, clk: u32) {
        /* Set clock */
        self.mmc_host_info_mut().set_clock(clk);
        let host_info = self.mmc_host_info().clone();

        self.host_ops_mut().mmc_set_ios(&host_info)
    }

    pub fn mmc_card_hs400es(&self) -> bool {
        let timing = self.mmc_host_info().timing;
        timing == MMC_TIMING_MMC_HS400ES
    }

    /// check if the card is in HS200 mode
    pub fn mmc_card_hs200(&self) -> bool {
        let timing = self.mmc_host_info().timing;
        timing == MMC_TIMING_MMC_HS200
    }

    /// check if the card supports high-speed modes
    fn mmc_card_hs(&self) -> bool {
        let timing = self.mmc_host_info().timing;
        (timing == MMC_TIMING_MMC_HS) || (timing == MMC_TIMING_SD_HS)
    }

    pub fn mmc_set_bus_speed(&mut self, avail_type: u32) {
        let clock = if self.mmc_card_hs() {
            if (avail_type & EXT_CSD_CARD_TYPE_52 as u32) != 0 {
                MMC_HIGH_52_MAX_DTR
            } else {
                MMC_HIGH_26_MAX_DTR
            }
        } else if self.mmc_card_hs200() {
            MMC_HS200_MAX_DTR
        } else {
            0 // Default clock value when no condition matches
        };

        self.mmc_set_clock(clock);
    }

    pub fn mmc_select_card_type(&self, ext_csd: &[u8], host_caps: u32) -> u16 {
        let card_type = ext_csd[EXT_CSD_CARD_TYPE as usize] as u16;
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

    /// Send a single tuning block read command over the SDHCI interface
    fn mmc_send_tuning(&mut self, opcode: u8) -> MmcHostResult {
        // Helper to pack DMA boundary and block size fields
        let make_blksz = |dma: u16, blksz: u16| ((dma & 0x7) << 12) | (blksz & 0x0FFF);

        // Determine current bus width (1/4/8 bits)
        let bus_width = self.mmc_host_info().bus_width;

        // Choose block size: 128 bytes for HS200 on 8-bit bus, else 64 bytes
        let block_size = if opcode == MMC_SEND_TUNING_BLOCK_HS200 && bus_width == MMC_BUS_WIDTH_8BIT
        {
            128
        } else {
            64
        };

        // Program block size and enable DMA boundary
        self.host_ops()
            .write_reg16(EMMC_BLOCK_SIZE, make_blksz(7, block_size));
        // Set transfer mode to single-block read
        self.host_ops().write_reg16(EMMC_XFER_MODE, EMMC_TRNS_READ);

        // Build and send the tuning command
        let cmd = MmcCommand::new(opcode, 0, MMC_RSP_R1);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();

        Ok(())
    }

    /// Perform HS200 tuning sequence (also used for HS400 initial tuning)
    pub fn mmc_hs200_tuning(&mut self) -> MmcHostResult {
        let opcode = MMC_SEND_TUNING_BLOCK_HS200;
        let timing = self.mmc_host_info().timing;

        match timing {
            // HS400 tuning must be issued in HS200 mode; reject direct HS400 timing
            MMC_TIMING_MMC_HS400 => {
                return Err(MmcHostError::InvalidValue);
            }
            // HS200 timing: OK to proceed with tuning here
            MMC_TIMING_MMC_HS200 => {
                // HS400 re-tuning is not expected; leave periodic tuning disabled
            }
            // Any other timing mode is invalid for HS200 tuning
            _ => {
                return Err(MmcHostError::InvalidValue);
            }
        }

        // Set the EXEC_TUNING bit in Host Control2 to start tuning
        let mut ctrl = self.host_ops().read_reg16(EMMC_HOST_CTRL2);
        ctrl |= MMC_CTRL_EXEC_TUNING;
        self.host_ops().write_reg16(EMMC_HOST_CTRL2, ctrl);

        // Invoke the common tuning loop implementation
        self.__emmc_execute_tuning(opcode)
    }

    /// Core tuning loop: send tuning blocks until the controller indicates success or timeout
    fn __emmc_execute_tuning(&mut self, opcode: u8) -> MmcHostResult {
        const MAX_TUNING_LOOP: usize = 40;

        for _ in 0..MAX_TUNING_LOOP {
            // Send one tuning block command
            self.mmc_send_tuning(opcode)?;

            // Read back Host Control2 to check tuning status
            let ctrl = self.host_ops().read_reg16(EMMC_HOST_CTRL2);

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
        Err(MmcHostError::Timeout)
    }
}
