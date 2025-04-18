extern crate alloc;

mod block;
mod cmd;
mod config;
mod constant;
mod rockchip;

pub mod clock;

use crate::{delay_us, dump_memory_region, err::*, generic_fls};
use block::EMmcCard;
use clock::RK3568ClkPri;
use cmd::*;
use constant::*;
use core::{fmt::Display, sync::atomic::Ordering};
use log::{debug, info, warn};

#[derive(Debug, Clone, Copy)]
pub enum CardType {
    Unknown,
    Mmc,
    SdV1,
    SdV2,
    SdHc,
    MmcHc,
}

// SD Host Controller structure
#[derive(Debug)]
pub struct EMmcHost {
    base_addr: usize,
    card: Option<EMmcCard>,
    caps: u32,
    clock_base: u32,
}

impl Display for EMmcHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "EMMC Controller {{ base_addr: 0x{:#x}, card: {:?}, caps: {:#x}, clock_base: {} }}",
            self.base_addr, self.card, self.caps, self.clock_base
        )
    }
}

impl EMmcHost {
    pub fn new(base_addr: usize) -> Self {
        let mut host = Self {
            base_addr,
            card: None,
            caps: 0,
            clock_base: 0,
        };

        // Read capabilities
        host.caps = host.read_reg(EMMC_CAPABILITIES1);

        // Calculate base clock from capabilities
        host.clock_base = (host.caps >> 8) & 0xFF;
        host.clock_base *= 1000000; // convert to Hz

        info!("EMMC Controller created: {}", host);

        host
    }

    // Read a 32-bit register
    fn read_reg(&self, offset: u32) -> u32 {
        unsafe { core::ptr::read_volatile((self.base_addr + offset as usize) as *const u32) }
    }

    // Read a 16-bit register
    fn read_reg16(&self, offset: u32) -> u16 {
        unsafe { core::ptr::read_volatile((self.base_addr + offset as usize) as *const u16) }
    }

    // Read an 8-bit register
    fn read_reg8(&self, offset: u32) -> u8 {
        unsafe { core::ptr::read_volatile((self.base_addr + offset as usize) as *const u8) }
    }

    // Write a 32-bit register
    fn write_reg(&self, offset: u32, value: u32) {
        unsafe { core::ptr::write_volatile((self.base_addr + offset as usize) as *mut u32, value) }
    }

    // Write a 16-bit register
    fn write_reg16(&self, offset: u32, value: u16) {
        unsafe { core::ptr::write_volatile((self.base_addr + offset as usize) as *mut u16, value) }
    }

    // Write an 8-bit register
    fn write_reg8(&self, offset: u32, value: u8) {
        unsafe { core::ptr::write_volatile((self.base_addr + offset as usize) as *mut u8, value) }
    }

    pub fn cmd_error(&self) -> u32 {
        let mut errorstatus: u32 = CARD_OK;
        let mut timeout: u32 = MMC_MAX_CMD_TIMEOUT;
        while timeout > 0 {
            timeout -= 1;

            if self.sdhc_get_nisr_status(SDHC_NSR_CMD_COMPLETE) {
                self.sdhc_clear_nisr_status(SDHC_NSR_CMD_COMPLETE);
                break;
            }

            if self.sdhc_get_nisr_status(SDHC_NSR_ERR_INTR) {
                debug!(
                    "Cmd error, intr: {:x}, err: {:x}",
                    self.read_reg(EMMC_NORMAL_INT_STAT),
                    self.read_reg(EMMC_ERROR_INT_STAT)
                );

                let status = self.sdhc_get_eisr();
                if status != 0 {
                    self.sdhc_clear_eisr_status(status);
                    debug!("Error, SD_EISR: 0x{:04x}", status);
                    errorstatus = CARD_INTERNAL_ERROR;
                    return errorstatus;
                }
            }
        }

        if timeout == 0 {
            errorstatus = CARD_CMD_RSP_TIMEOUT;
            return errorstatus;
        }

        return errorstatus;
    }

    pub fn sdhc_get_nisr_status(&self, sdhc_nstatus: u32) -> bool {
        if self.read_reg(EMMC_NORMAL_INT_STAT) & sdhc_nstatus != 0 {
            return true;
        } else {
            return false;
        }
    }

    pub fn sdhc_clear_nisr_status(&self, sdhc_nstatus: u32) {
        self.write_reg(EMMC_NORMAL_INT_STAT, sdhc_nstatus);
    }

    pub fn sdhc_get_eisr(&self) -> u32 {
        self.read_reg(EMMC_ERROR_INT_STAT)
    }

    pub fn sdhc_clear_eisr_status(&self, sdhc_estatus: u32) {
        self.write_reg(EMMC_ERROR_INT_STAT, sdhc_estatus);
    }

    // Initialize the host controller
    pub fn init(&mut self, clk: &mut RK3568ClkPri) -> Result<(), SdError> {
        self.reset_all()?;
        //let mut errorstatus:u8 = CARD_OK;
        // 对于时钟，查看是否需要初始化
        debug!("emmc_get_clk: {}", clk.emmc_get_clk().unwrap());

        let _ = clk.emmc_set_clk(200_000_000);

        info!("Init EMMC Controller");

        let is_card_inserted = self.is_card_present();
        debug!("Card inserted: {}", is_card_inserted);

        let version = self.read_reg16(EMMC_HOST_CNTRL_VER);
        // version = 4.2
        info!("EMMC Version: 0x{:x}", version);

        let caps1 = self.read_reg(EMMC_CAPABILITIES1);
        info!("EMMC Capabilities 1: 0b{:b}", caps1);

        let mut clk_mul: u32 = 0;

        if (version & EMMC_SPEC_VER_MASK) >= EMMC_SPEC_300 {
            let caps2 = self.read_reg(EMMC_CAPABILITIES2);
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

        info!("voltage range: {:#x}", generic_fls(voltages as u32) - 1);

        // Reset the controller
        //self.reset_all()?;

        // Perform full power cycle
        //self.sdhci_set_power(generic_fls(voltages as u32) - 1)?;

        // Set bus width to 1-bit
        let mut ctrl1 = self.read_reg8(EMMC_HOST_CTRL1);
        debug!("ctrl1: {:#x}", ctrl1);
        self.write_reg8(EMMC_HOST_CTRL1, ctrl1 & EMMC_HC0_WIDTH_MASK);
        ctrl1 = self.read_reg8(EMMC_HOST_CTRL1);
        debug!("ctrl1: {:#x}", ctrl1);
        self.write_reg8(EMMC_HOST_CTRL1, ctrl1 | EMMC_CTRL_1BITBUS);
        debug!("ctrl1: {:#x}", ctrl1);

        // set speed mode
        ctrl1 = self.read_reg8(EMMC_HOST_CTRL1);
        debug!("ctrl1: {:#x}", ctrl1);
        ctrl1 &= EMMC_HC0_SPEED_MASK;
        ctrl1 |= EMMC_HC0_HI_SPEED_EN;
        self.write_reg8(EMMC_HOST_CTRL1, ctrl1);

        ctrl1 = self.read_reg8(EMMC_HOST_CTRL1);
        debug!("ctrl1: {:#x}", ctrl1);

        //
        let mut ctrl2 = self.read_reg16(EMMC_HOST_CTRL2);
        ctrl2 &= EMMC_HC1_1V8SIG_SPEEDMODE_MASK;
        ctrl2 |= EMMC_MODE_LEGACY_COMPATIBLE_3V & 0x07;
        self.write_reg16(EMMC_HOST_CTRL2, ctrl2);

        //
        let mut pwr = self.read_reg8(EMMC_POWER_CTRL);
        pwr &= EMMC_HC0_SPEED_MASK;
        pwr |= EMMC_POWER_STATE_1V8;
        // self.write_reg(EMMC_POWER_CTRL, 0xf);
        //let pwr = self.read_reg(EMMC_POWER_CTRL);
        info!("pwr {:#x}", pwr);
        self.write_reg8(EMMC_POWER_CTRL, pwr);
        delay_us(2000000);

        // Set initial clock and wait for it to stabilize
        debug!("emmc_get_clk {}", clk.emmc_get_clk().unwrap());
        self.dwcmshc_sdhci_emmc_set_clock(400000, clk)?; // Start with 400 KHz for initialization

        //self.write_reg16(EMMC_HOST_CTRL2, 0);

        // Enable interrupts
        self.write_reg(
            EMMC_NORMAL_INT_STAT_EN,
            EMMC_INT_CMD_MASK | EMMC_INT_DATA_MASK,
        );
        self.write_reg(EMMC_SIGNAL_ENABLE, 0x0);

        // let addr = 0xfffff000fe310000;
        // let size = 0x1000;
        // unsafe { dump_memory_region(addr, size) };

        // Check if card is present
        if !self.is_card_present() {
            return Err(SdError::NoCard);
        }

        // unsafe {
        //     dump_memory_region(self.base_addr, 0x1000);
        // }

        // Initialize the card
        self.init_card()?;

        info!("EMMC initialization completed successfully");
        Ok(())
    }

    // Reset the controller
    fn reset_all(&self) -> Result<(), SdError> {
        // Request reset
        self.write_reg8(EMMC_SOFTWARE_RESET, EMMC_RESET_ALL);

        // Wait for reset to complete with timeout
        let mut timeout = 20000; // Increased timeout
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & EMMC_RESET_ALL) != 0 {
            if timeout == 0 {
                return Err(SdError::Timeout);
            }
            timeout -= 1;
        }

        Ok(())
    }

    // Reset data line
    fn reset_data(&self) -> Result<(), SdError> {
        self.write_reg8(EMMC_SOFTWARE_RESET, EMMC_RESET_DATA);

        // Wait for reset to complete
        let mut timeout = 100000;
        while (self.read_reg8(EMMC_SOFTWARE_RESET) & EMMC_RESET_DATA) != 0 {
            if timeout == 0 {
                return Err(SdError::Timeout);
            }
            timeout -= 1;
        }

        Ok(())
    }

    // Check if card is present
    fn is_card_present(&self) -> bool {
        let state = self.read_reg(EMMC_PRESENT_STATE);
        (state & EMMC_CARD_INSERTED) != 0
    }

    // Check if card is write protected
    fn is_write_protected(&self) -> bool {
        let state = self.read_reg(EMMC_PRESENT_STATE);
        (state & EMMC_WRITE_PROTECT) != 0
    }

    // Initialize the eMMC card
    fn init_card(&mut self) -> Result<(), SdError> {
        info!("eMMC initialization started");
        let mut errorstatus: u32 = CARD_OK;

        // Create card structure
        let mut card = EMmcCard::init(self.base_addr, CardType::Mmc);

        self.mmc_go_idle()?;
        errorstatus = self.cmd_error();
        if errorstatus != CARD_OK {
            warn!("CMD0 Error, Status: 0X{:x}", errorstatus);
        }

        delay_us(2000000);

        
        

        self.mmc_send_op_cond(&mut card, 0x40ff8080, 5)?;
        errorstatus = self.cmd_error();
        if errorstatus != CARD_OK {
            warn!("CMD0 Error, Status: 0X{:x}", errorstatus);
        }
        debug!("CMD1 response : 0x{:x}", self.get_response().as_r3());

        if self.get_response().as_r3() & 0x7F != 0 {
            warn!("Error, card support voltages below defined range");
        }
        let respone = self.get_response().as_r3();

        if respone & 0x40000000 != 0 {
            debug!("Multimedia Card is Sector Mode");
        }

        // // Send CMD0 to reset the card
        // self.mmc_go_idle()?;

        // // Send CMD1 to set OCR and check if card is ready
        // self.mmc_send_op_cond(&mut card, ocr, retry)?;

        // Send CMD2 to get CID
        self.mmc_all_send_cid(&mut card)?;

        // // Send CMD3 to get RCA
        self.mmc_set_relative_addr(&mut card)?;

        // // Send CMD9 to get CSD
        self.mmc_send_csd(&mut card)?;

        // Card is initialized
        card.initialized.store(true, Ordering::SeqCst);
        card.state |= MMC_STATE_PRESENT;

        // Store the card in the host
        self.card = Some(card);

        Ok(())
    }

    // Send CMD0 to reset the card
    fn mmc_go_idle(&self) -> Result<(), SdError> {
        let cmd = EMmcCommand::new(MMC_GO_IDLE_STATE, 0, MMC_RSP_NONE);
        self.send_command(&cmd)?;

        delay_us(2000000);

        info!("eMMC reset complete");
        Ok(())
    }

    // Send CMD1 to set OCR and check if card is ready
    fn mmc_send_op_cond(
        &self,
        card: &mut EMmcCard,
        ocr: u32,
        mut retry: u32,
    ) -> Result<(), SdError> {

        self.mmc_go_idle()?;
        delay_us(2000000);

        info!(
            "mmc_send_op_cond: Power Status {:b}",
            self.read_reg8(EMMC_POWER_CTRL)
        );

        // First iteration - send without args to query capabilities
        let mut cmd = EMmcCommand::new(MMC_SEND_OP_COND, 0, MMC_RSP_R3);
        self.send_command(&cmd)?;

        card.ocr = self.get_response().as_r3();

        info!(
            "CMD1 sent, Present State: {:#x}",
            self.read_reg(EMMC_PRESENT_STATE)
        );

        debug!("eMMC first CMD1 response (no args): {:#x}", card.ocr);

        // Now retry with the proper argument until ready or timeout
        let mut ready = false;
        while retry > 0 && !ready {
            cmd = EMmcCommand::new(MMC_SEND_OP_COND, ocr, MMC_RSP_R3);
            self.send_command(&cmd)?;
            card.ocr = self.get_response().as_r3();

            info!(
                "CMD1 sent, Present State: {:#x}",
                self.read_reg(EMMC_PRESENT_STATE)
            );
            info!("CMD1 response raw: {:#x}", self.read_reg(EMMC_RESPONSE));

            info!("eMMC CMD1 response: {:#x}", card.ocr);

            // // Check if card is ready (OCR_BUSY flag set)
            // if card.ocr >> 31 == 1 {
            //     ready = true;
            //     //if (card.ocr & ocr_hcs) != 0 {
            //     card.card_type = CardType::MmcHc;
            //     card.state |= MMC_STATE_HIGHCAPACITY;
            //     //}
            // } else {
            //     retry -= 1;
            //     // Delay between retries
            //     delay_us(20);
            // }
            retry -= 1;
            // Delay between retries
            delay_us(3000000);

        }

        info!("eMMC initialization status: {}", ready);

        if !ready {
            return Err(SdError::UnsupportedCard);
        }

        delay_us(1000);

        debug!(
            "Clock control before CMD2: 0x{:x}, stable: {}",
            self.read_reg16(EMMC_CLOCK_CONTROL),
            self.is_clock_stable()
        );

        Ok(())
    }

    // Send CMD2 to get CID
    fn mmc_all_send_cid(&self, card: &mut EMmcCard) -> Result<(), SdError> {
        let cmd: EMmcCommand = EMmcCommand::new(MMC_ALL_SEND_CID, 0, MMC_RSP_R2);
        self.send_command(&cmd)?;
        debug!("sent command.");
        let response = self.get_response();
        card.cid = response.as_r2();

        info!(
            "eMMC Card CID: {:b} {:b} {:b} {:b}",
            response.as_r2()[0],
            response.as_r2()[1],
            response.as_r2()[2],
            response.as_r2()[3]
        );

        // For eMMC, host assigns the RCA value (unlike SD where card provides it)
        let mmc_rca = 0x0002 << 16; // Typical RCA value for eMMC is 1
        card.rca = mmc_rca;
        Ok(())
    }

    // Send CMD3 to set RCA for eMMC
    fn mmc_set_relative_addr(&self, card: &mut EMmcCard) -> Result<(), SdError> {
        let cmd = EMmcCommand::new(MMC_SET_RELATIVE_ADDR, card.rca, MMC_RSP_R1);
        self.send_command(&cmd)?;
        Ok(())
    }

    // Send CMD9 to get CSD
    fn mmc_send_csd(&self, card: &mut EMmcCard) -> Result<(), SdError> {
        let cmd = EMmcCommand::new(MMC_SEND_CSD, card.rca, MMC_RSP_R2);
        self.send_command(&cmd)?;
        let response = self.get_response();
        card.csd = response.as_r2();

        info!(
            "eMMC Card info: CSD: {:b} {:b} {:b} {:b}",
            response.as_r2()[0],
            response.as_r2()[1],
            response.as_r2()[2],
            response.as_r2()[3]
        );

        // Calculate card capacity from CSD
        let csd_version = (card.csd[3] >> 22) & 0x3;
        debug!("eMMC CSD version: {}", csd_version);
        Ok(())
    }

    // Helper function to check if controller supports 8-bit bus
    fn supports_8bit_bus(&self) -> bool {
        // Read controller capabilities register
        // This is a placeholder - actual implementation depends on your EMMC controller
        let caps = self.read_reg(EMMC_CAPABILITIES1);
        (caps & EMMC_CAN_DO_8BIT) != 0
    }

    // Get card status
    pub fn get_status(&self) -> Result<u32, SdError> {
        // Check if card is initialized
        let card = match &self.card {
            Some(card) => card,
            None => return Err(SdError::NoCard),
        };

        if !card.initialized.load(Ordering::SeqCst) {
            return Err(SdError::UnsupportedCard);
        }

        // Send SEND_STATUS command
        let cmd = EMmcCommand::new(MMC_SEND_STATUS, card.rca, MMC_RSP_R1);
        self.send_command(&cmd)?;
        let response = self.get_response();

        Ok(response.as_r1())
    }

    // Get card info
    pub fn get_card_info(&self) -> Result<CardInfo, SdError> {
        // Check if card is initialized
        let card = match &self.card {
            Some(card) => card,
            None => return Err(SdError::NoCard),
        };

        if !card.initialized.load(Ordering::SeqCst) {
            return Err(SdError::UnsupportedCard);
        }

        // Extract information from CID
        let cid = card.cid;

        // SD card CID format
        let manufacturer_id = (cid[0] >> 24) as u8;
        let application_id = ((cid[0] >> 8) & 0xFFFF) as u16;
        let serial_number = ((cid[0] & 0xFF) << 24) | ((cid[1] >> 8) & 0xFFFFFF);

        // Extract manufacturing date
        let manufacturing_year = (((cid[1] & 0xF) << 4) | ((cid[2] >> 28) & 0xF)) as u16 + 2000;
        let manufacturing_month = ((cid[2] >> 24) & 0xF) as u8;

        let card_info = CardInfo {
            card_type: card.card_type,
            manufacturer_id,
            application_id,
            serial_number,
            manufacturing_month,
            manufacturing_year,
            capacity_bytes: card.capacity_blocks * 512,
            block_size: 512,
        };

        Ok(card_info)
    }

    // Get card capacity in bytes
    pub fn get_capacity(&self) -> Result<u64, SdError> {
        // Check if card is initialized
        let card = match &self.card {
            Some(card) => card,
            None => return Err(SdError::NoCard),
        };

        if !card.initialized.load(Ordering::SeqCst) {
            return Err(SdError::UnsupportedCard);
        }

        Ok(card.capacity_blocks * 512)
    }
}

// Card information structure
#[derive(Debug)]
pub struct CardInfo {
    pub card_type: CardType,
    pub manufacturer_id: u8,
    pub application_id: u16,
    pub serial_number: u32,
    pub manufacturing_month: u8,
    pub manufacturing_year: u16,
    pub capacity_bytes: u64,
    pub block_size: u32,
}
