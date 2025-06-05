pub mod block;

mod cmd;
mod ext;
mod sd;
mod mmc;

extern crate alloc;
use alloc::string::String;
use log::debug;
use log::info;
use log::trace;

use alloc::vec::Vec;
use core::fmt::Debug;
use core::panic;

use super::common::commands::MmcCommand;
use crate::common::HostCapabilities;
use crate::{
    aux::*,
    card::{CardType, MmcCard},
    constants::*,
    delay_us,
    host::{MmcHostError, MmcHostOps, MmcHostResult},
};

#[derive(Debug)]
pub struct UDevice {
    pub name: String,
    pub compatible: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MmcHostInfo {
    pub timing: u32,
    pub bus_width: u8,
    pub clock: u32,
}

impl MmcHostInfo {
    pub fn new() -> Self {
        MmcHostInfo {
            timing: MMC_TIMING_LEGACY,
            bus_width: 1,  // Default to 1-bit bus width
            clock: 400000, // Default to 400 kHz
        }
    }

    pub fn set_timing(&mut self, timing: u32) {
        self.timing = timing;
    }

    pub fn set_bus_width(&mut self, bus_width: u8) {
        self.bus_width = bus_width;
    }

    pub fn set_clock(&mut self, clock: u32) {
        self.clock = clock;
    }
}

pub struct MmcHost<T: MmcHostOps> {
    pub name: String,
    pub card: Option<MmcCard>,
    pub host_info: MmcHostInfo,
    pub host_ops: T,
}

impl<T: MmcHostOps> MmcHost<T> {
    pub fn new(name: String, host_ops: T) -> Self {
        MmcHost {
            host_info: MmcHostInfo::new(),
            name,
            card: None,
            host_ops,
        }
    }

    pub fn init(&mut self) -> MmcHostResult {
        info!("eMMC host initialization started");

        self.host_ops_mut().init_host().unwrap();

        // Set initial bus width to 1-bit
        self.mmc_set_bus_width(MMC_BUS_WIDTH_1BIT);

        // Set initial clock and wait for it to stabilize
        self.mmc_set_clock(400000);

        self.mmc_set_timing(MMC_TIMING_LEGACY);

        if self.is_card_present() {
            let mmc_card = MmcCard::new();
            self.add_card(mmc_card);
            self.init_card()?;
        }

        info!("eMMC host initialization complete");
        Ok(())
    }

    fn add_card(&mut self, card: MmcCard) {
        self.card = Some(card);
    }

    fn card(&self) -> Option<&MmcCard> {
        self.card.as_ref()
    }

    fn card_mut(&mut self) -> Option<&mut MmcCard> {
        self.card.as_mut()
    }

    fn host_ops(&self) -> &T {
        &self.host_ops
    }

    fn host_ops_mut(&mut self) -> &mut T {
        &mut self.host_ops
    }

    fn mmc_host_info(&self) -> &MmcHostInfo {
        &self.host_info
    }

    fn mmc_host_info_mut(&mut self) -> &mut MmcHostInfo {
        &mut self.host_info
    }

    // Check if card is present
    fn is_card_present(&self) -> bool {
        let state = self.host_ops().read_reg32(EMMC_PRESENT_STATE);
        // debug!("EMMC Present State: {:#b}", state);
        (state & EMMC_CARD_INSERTED) != 0 && ((state & EMMC_CARD_STABLE) != 0)
    }

    pub fn mmc_poll_for_busy(&self, send_status: bool) -> MmcHostResult {
        let mut busy = true;
        let mut timeout = 1000;
        let rca = self.card().unwrap().base_info().rca();

        while busy {
            if send_status {
                let cmd = MmcCommand::new(MMC_SEND_STATUS, rca << 16, MMC_RSP_R1);
                self.host_ops().mmc_send_command(&cmd, None).unwrap();
                let response = self.get_response().as_r1();
                trace!("cmd_d {:#x}", response);

                if response & MMC_STATUS_SWITCH_ERROR != 0 {
                    return Err(MmcHostError::CommandError);
                }
                busy = (response & MMC_STATUS_CURR_STATE) == MMC_STATE_PRG;
                if !busy {
                    break;
                }
            } else {
                busy = self.host_ops().mmc_card_busy();
            }

            if timeout == 0 && busy {
                return Err(MmcHostError::Timeout);
            }

            timeout -= 1;
            delay_us(1000);
        }

        Ok(())
    }

    fn mmc_select_hs(&mut self) -> MmcHostResult {
        let ret = self.mmc_switch(
            EXT_CSD_CMD_SET_NORMAL,
            EXT_CSD_HS_TIMING,
            EXT_CSD_TIMING_HS,
            true,
        );

        if ret.is_ok() {
            // self.mmc_set_timing(MMC_TIMING_MMC_HS);
        }

        ret
    }

    pub fn mmc_select_card(&mut self) -> MmcHostResult {
        // Get the RCA value before borrowing the card
        let card = self.card().unwrap();
        let rca = card.base_info().rca();

        // CMD7: Select the card
        let cmd = MmcCommand::new(MMC_SELECT_CARD, rca << 16, MMC_RSP_R1);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();

        debug!("cmd7: {:#x}", self.get_response().as_r1());

        Ok(())
    }

    // Initialize the eMMC card
    fn init_card(&mut self) -> MmcHostResult {
        info!("eMMC initialization started");

        // CMD0: Put card into idle state
        self.mmc_go_idle()?;

        // CMD1: Send operation condition (OCR) and wait for card ready
        let ocr = 0x00; // Voltage window: 2.7V to 3.6V
        let retry = 100;

        let voltages = {
            let capabilities = self.host_ops().get_capabilities();
            capabilities.get_voltages()
        };
        let ocr = self.mmc_send_op_cond(ocr, retry, voltages)?;
        let high_capacity = (ocr & OCR_HCS) == OCR_HCS;

        let card_mut = self.card_mut().unwrap();

        // Set RCA (Relative Card Address)
        card_mut.base_info_mut().set_rca(1);

        // Determine if card is high capacity (SDHC/SDXC/eMMC)
        card_mut.base_info_mut().set_high_capacity(high_capacity);

        // CMD2: Request CID (Card Identification)
        let _cid = self.mmc_all_send_cid()?;

        // CMD3: Set RCA and switch card to "standby" state
        self.mmc_set_relative_addr()?;

        // CMD9: Read CSD (Card-Specific Data) register
        let csd = self.mmc_send_csd()?;

        // Determine card version from CSD if unknown
        self.parse_csd_and_set_version(&csd)?;

        // Extract parameters from CSD for frequency, size, and block lengths
        let (freq, mult, mut read_bl_len, mut write_bl_len, csize, cmult) = {
            let card_mut = self.card_mut().unwrap();
            let freq = FBASE[(csd[0] & 0x7) as usize];
            let mult = MULTIPLIERS[((csd[0] >> 3) & 0xf) as usize];
            let read_bl_len = 1 << ((csd[1] >> 16) & 0xf);
            let card_type = card_mut.card_type();
            let write_bl_len = if card_type == CardType::Mmc {
                1 << ((csd[3] >> 22) & 0xf)
            } else if card_type == CardType::SdV1 || card_type == CardType::SdV2 {
                read_bl_len
            } else {
                panic!(
                    "Unsupported card type for write block length: {:?}",
                    card_type
                );
            };

            let high_capacity = card_mut.base_info().high_capacity();
            let (csize, cmult) = if high_capacity {
                ((csd[1] & 0x3f) << 16 | (csd[2] & 0xffff0000) >> 16, 8)
            } else {
                (
                    (csd[1] & 0x3ff) << 2 | (csd[2] & 0xc0000000) >> 30,
                    (csd[2] & 0x00038000) >> 15,
                )
            };
            (freq, mult, read_bl_len, write_bl_len, csize, cmult)
        };

        // Calculate user capacity
        let _tran_speed = freq * mult as usize;
        let mut capacity_user = (csize as u64 + 1) << (cmult as u64 + 2);
        capacity_user *= read_bl_len as u64;

        let mut capacity_gp = [0; 4];

        // Clip read/write block lengths to max supported size
        if write_bl_len > MMC_MAX_BLOCK_LEN {
            write_bl_len = MMC_MAX_BLOCK_LEN;
        }
        if read_bl_len > MMC_MAX_BLOCK_LEN {
            read_bl_len = MMC_MAX_BLOCK_LEN;
        }

        let card_mut = self.card_mut().unwrap();
        card_mut.base_info_mut().set_read_bl_len(read_bl_len);
        card_mut.base_info_mut().set_write_bl_len(write_bl_len);

        // Set initial erase group size and partition config
        card_mut.base_info_mut().set_erase_grp_size(1);
        card_mut
            .base_info_mut()
            .set_part_config(MMCPART_NOAVAILABLE);

        let dsr_imp = ((csd[1] >> 12) & 0x1) != 0;
        card_mut.base_info_mut().set_dsr_imp(dsr_imp);
        
        // CMD4: Set DSR if required by card
        self.set_dsr_if_required()?;

        self.mmc_select_card()?;

        // For eMMC 4.0+, configure high-speed, EXT_CSD and partitions
        if self.is_emmc_version_4_plus() {
            self.mmc_select_hs()?; // Switch to high speed
            self.mmc_set_clock(MMC_HIGH_52_MAX_DTR); // Set high-speed clock

            // Allocate buffer for EXT_CSD read
            cfg_if::cfg_if! {
                if #[cfg(feature = "dma")] {
                    let mut ext_csd: DVec<u8> = DVec::zeros(MMC_MAX_BLOCK_LEN as usize, 0x1000, Direction::FromDevice).unwrap();
                } else if #[cfg(feature = "pio")] {
                    let mut ext_csd: [u8; 512] = [0; 512];
                }
            }

            // CMD8: Read EXT_CSD
            self.mmc_send_ext_csd(&mut ext_csd)?;
            let mut ext_csd = ext_csd.to_vec();
            trace!("EXT_CSD: {:?}", ext_csd);

            // Extract capacity and version
            if ext_csd[EXT_CSD_REV as usize] >= 2 {
                let mut capacity: u64 = ext_csd[EXT_CSD_SEC_CNT as usize] as u64
                    | (ext_csd[EXT_CSD_SEC_CNT as usize + 1] as u64) << 8
                    | (ext_csd[EXT_CSD_SEC_CNT as usize + 2] as u64) << 16
                    | (ext_csd[EXT_CSD_SEC_CNT as usize + 3] as u64) << 24;
                capacity *= MMC_MAX_BLOCK_LEN as u64;
                if (capacity >> 20) > 2 * 1024 {
                    capacity_user = capacity;
                }

                let card = self.card_mut().unwrap();
                match ext_csd[EXT_CSD_REV as usize] {
                    1 => card.base_info_mut().set_card_version(MMC_VERSION_4_1),
                    2 => card.base_info_mut().set_card_version(MMC_VERSION_4_2),
                    3 => card.base_info_mut().set_card_version(MMC_VERSION_4_3),
                    5 => card.base_info_mut().set_card_version(MMC_VERSION_4_41),
                    6 => card.base_info_mut().set_card_version(MMC_VERSION_4_5),
                    7 => card.base_info_mut().set_card_version(MMC_VERSION_5_0),
                    8 => card.base_info_mut().set_card_version(MMC_VERSION_5_1),
                    _ => panic!("Unknown EXT_CSD revision"),
                }
            }

            // Parse partition configuration info
            let part_completed = (ext_csd[EXT_CSD_PARTITION_SETTING as usize] as u32
                & EXT_CSD_PARTITION_SETTING_COMPLETED)
                != 0;

            if (ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize] as u32 & PART_SUPPORT != 0)
                || ext_csd[EXT_CSD_BOOT_MULT as usize] != 0
            {
                self.card_mut()
                    .unwrap()
                    .base_info_mut()
                    .set_part_config(ext_csd[EXT_CSD_PART_CONF as usize]);
            }

            // Check secure erase support
            if ext_csd[EXT_CSD_SEC_FEATURE_SUPPORT as usize] as u32 & EXT_CSD_SEC_GB_CL_EN != 0 {
                let _mmc_can_trim = 1;
            }

            // Calculate boot and RPMB sizes
            let capacity_boot = (ext_csd[EXT_CSD_BOOT_MULT as usize] as u64) << 17;
            let capacity_rpmb = (ext_csd[EXT_CSD_RPMB_MULT as usize] as u64) << 17;

            debug!("Boot partition size: {:#x}", capacity_boot);
            debug!("RPMB partition size: {:#x}", capacity_rpmb);

            // Calculate general purpose partition sizes
            let mut has_parts = false;
            for i in 0..4 {
                let idx = EXT_CSD_GP_SIZE_MULT as usize + i * 3;
                let mult = ((ext_csd[idx + 2] as u32) << 16)
                    + ((ext_csd[idx + 1] as u32) << 8)
                    + (ext_csd[idx] as u32);
                if mult != 0 {
                    has_parts = true;
                }
                if !part_completed {
                    continue;
                }
                capacity_gp[i] = mult as u64;
                capacity_gp[i] *= ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u64;
                capacity_gp[i] *= ext_csd[EXT_CSD_HC_WP_GRP_SIZE as usize] as u64;
                capacity_gp[i] <<= 19;
            }
            debug!("GP partition sizes: {:?}", capacity_gp);

            let (mut enh_user_size, mut enh_user_start) = (0, 0);
            // Calculate enhanced user data size and start
            if part_completed {
                enh_user_size = ((ext_csd[EXT_CSD_ENH_SIZE_MULT as usize + 2] as u64) << 16)
                    + ((ext_csd[EXT_CSD_ENH_SIZE_MULT as usize + 1] as u64) << 8)
                    + (ext_csd[EXT_CSD_ENH_SIZE_MULT as usize] as u64);
                enh_user_size *= ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u64;
                enh_user_size *= ext_csd[EXT_CSD_HC_WP_GRP_SIZE as usize] as u64;
                enh_user_size <<= 19;

                enh_user_start = ((ext_csd[EXT_CSD_ENH_START_ADDR as usize + 3] as u64) << 24)
                    + ((ext_csd[EXT_CSD_ENH_START_ADDR as usize + 2] as u64) << 16)
                    + ((ext_csd[EXT_CSD_ENH_START_ADDR as usize + 1] as u64) << 8)
                    + (ext_csd[EXT_CSD_ENH_START_ADDR as usize] as u64);
                if high_capacity {
                    enh_user_start <<= 9;
                }
            }

            // If partitions are configured, enable ERASE_GRP_DEF
            if part_completed {
                has_parts = true;
            }

            if (ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize] as u32 & PART_SUPPORT != 0)
                && (ext_csd[EXT_CSD_PARTITIONS_ATTRIBUTE as usize] as u32 & PART_ENH_ATTRIB != 0)
            {
                has_parts = true;
            }

            if has_parts {
                let err = self.mmc_switch(EXT_CSD_CMD_SET_NORMAL, EXT_CSD_ERASE_GROUP_DEF, 1, true);
                if err.is_err() {
                    return Err(MmcHostError::CommandError);
                } else {
                    ext_csd[EXT_CSD_ERASE_GROUP_DEF as usize] = 1;
                }
            }

            // Calculate erase group size
            if ext_csd[EXT_CSD_ERASE_GROUP_DEF as usize] & 0x01 != 0 {
                self.card_mut().unwrap().base_info_mut().set_erase_grp_size(
                    (ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u32) * 1024,
                );

                let high_capacity = self.card().unwrap().base_info().high_capacity();
                if high_capacity && part_completed {
                    let capacity = (ext_csd[EXT_CSD_SEC_CNT as usize] as u64)
                        | ((ext_csd[EXT_CSD_SEC_CNT as usize + 1] as u64) << 8)
                        | ((ext_csd[EXT_CSD_SEC_CNT as usize + 2] as u64) << 16)
                        | ((ext_csd[EXT_CSD_SEC_CNT as usize + 3] as u64) << 24);
                    capacity_user = capacity * (MMC_MAX_BLOCK_LEN as u64);
                }
            } else {
                let erase_gsz = (csd[2] & 0x00007c00) >> 10;
                let erase_gmul = (csd[2] & 0x000003e0) >> 5;
                self.card_mut()
                    .unwrap()
                    .base_info_mut()
                    .set_erase_grp_size((erase_gsz + 1) * (erase_gmul + 1));
            }

            // Set high-capacity write-protect group size
            let hc_wp_grp_size = 1024
                * (ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u64)
                * (ext_csd[EXT_CSD_HC_WP_GRP_SIZE as usize] as u64);

            let mmc_ext = self
                .card_mut()
                .unwrap()
                .cardext_mut()
                .unwrap()
                .as_mut_mmc()
                .unwrap();
            mmc_ext.part_support = ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize];
            mmc_ext.capacity_boot = capacity_boot;
            mmc_ext.capacity_rpmb = capacity_rpmb;
            mmc_ext.capacity_gp = capacity_gp;
            mmc_ext.hc_wp_grp_size = hc_wp_grp_size;
            mmc_ext.capacity_user = capacity_user;

            // Set write reliability and drive strength
            mmc_ext.wr_rel_set = ext_csd[EXT_CSD_WR_REL_SET as usize];
            mmc_ext.raw_driver_strength = ext_csd[EXT_CSD_DRIVER_STRENGTH as usize];

            if part_completed {
                // Save enhanced partition attributes
                if ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize] as u32 & ENHNCD_SUPPORT != 0 {
                    mmc_ext.part_attr = ext_csd[EXT_CSD_PARTITIONS_ATTRIBUTE as usize];
                }
                mmc_ext.enh_user_size = enh_user_size;
                mmc_ext.enh_user_start = enh_user_start;
            }
        }

        // Final configuration
        self.finalize_card_initialization()?;

        Ok(())
    }

    // Set DSR if required by card
    fn set_dsr_if_required(&mut self) -> MmcHostResult {
        let card_mut = self.card_mut().unwrap();
        let (dsr_needed, dsr) = {
            let dsr = card_mut.base_info().dsr();
            let dsr_imp = card_mut.base_info().dsr_imp();
            ((dsr_imp as u8 != 0) && 0xffffffff != dsr, dsr)
        };

        if dsr_needed {
            let dsr_value = (dsr & 0xffff) << 16;
            self.mmc_set_dsr(dsr_value)?;
        }
        
        Ok(())
    }

    // Determine card version from CSD if unknown
    fn parse_csd_and_set_version(&mut self, csd: &[u32; 4]) -> MmcHostResult {
        let card_version = {
            let card_mut = self.card_mut().unwrap();
            card_mut.base_info().card_version()
        };

        if card_version == MMC_VERSION_UNKNOWN {
            let card_mut = self.card_mut().unwrap();
            let csd_version = (csd[0] >> 26) & 0xf;
            debug!("eMMC CSD version: {}", csd_version);
            match csd_version {
                0 => card_mut.base_info_mut().set_card_version(MMC_VERSION_1_2),
                1 => card_mut.base_info_mut().set_card_version(MMC_VERSION_1_4),
                2 => card_mut.base_info_mut().set_card_version(MMC_VERSION_2_2),
                3 => card_mut.base_info_mut().set_card_version(MMC_VERSION_3),
                4 => card_mut.base_info_mut().set_card_version(MMC_VERSION_4),
                _ => card_mut.base_info_mut().set_card_version(MMC_VERSION_1_2),
            }
        }
        
        Ok(())
    }

    // Check if card is eMMC 4.0+
    fn is_emmc_version_4_plus(&self) -> bool {
        let card = self.card().unwrap();
        let card_type = card.card_type();
        card_type == CardType::Mmc && card.base_info().card_version() >= MMC_VERSION_4
    }

    // Final initialization steps
    fn finalize_card_initialization(&mut self) -> MmcHostResult {
        let host_caps = {
            let capabilities = self.host_ops().get_capabilities();
            capabilities.get_host_caps()
        };
        
        self.mmc_set_capacity(0)?;
        self.mmc_change_freq(host_caps)?;
        self.card_mut().unwrap().set_initialized(true);

        Ok(())
    }

    fn mmc_set_capacity(&mut self, part_num: u32) -> MmcHostResult {
        // part_num 暂时设置为 0
        let card_mut = self.card_mut().unwrap();
        let mmc_ext = card_mut.cardext_mut().unwrap().as_mut_mmc().unwrap();
        match part_num {
            0 => {
                let capacity_user = mmc_ext.capacity_user;
                card_mut.base_info_mut().set_capacity(capacity_user);
            }
            1 | 2 => {
                let capacity_boot = mmc_ext.capacity_boot;
                card_mut.base_info_mut().set_capacity(capacity_boot);
            }
            3 => {
                let capacity_rpmb = mmc_ext.capacity_rpmb;
                card_mut.base_info_mut().set_capacity(capacity_rpmb);
            }
            4..=7 => {
                let capacity_gp = mmc_ext.capacity_gp;
                card_mut
                    .base_info_mut()
                    .set_capacity(capacity_gp[(part_num - 4) as usize]);
            }
            _ => return Err(MmcHostError::InvalidValue),
        }

        let capacity = card_mut.base_info().capacity();
        let read_bl_len = card_mut.base_info().read_bl_len();
        let _lba = lldiv(capacity, read_bl_len);

        Ok(())
    }

    pub fn mmc_change_freq(&mut self, host_caps: u32) -> MmcHostResult {
        let card_mut = self.card_mut().unwrap();
        // let mmc_ext = card_mut.cardext_mut().unwrap().as_mut_mmc().unwrap();
        // Allocate buffer for EXT_CSD depending on whether DMA or PIO is enabled
        cfg_if::cfg_if! {
            if #[cfg(feature = "dma")] {
                let mut ext_csd: DVec<u8> = DVec::zeros(MMC_MAX_BLOCK_LEN as usize, 0x1000, Direction::FromDevice).unwrap();
            } else if #[cfg(feature = "pio")] {
                let mut ext_csd: [u8; 512] = [0; 512];
            }
        }

        // Initialize card capabilities flags
        card_mut.base_info_mut().set_card_caps(0);

        // Get card version (default to 0 if not available)
        let version = card_mut.base_info().card_version();

        // Only cards version 4.0 and above support high-speed modes
        if version < MMC_VERSION_4 {
            return Ok(());
        }

        // Enable both 4-bit and 8-bit modes on the card
        card_mut
            .base_info_mut()
            .set_card_caps(MMC_MODE_4BIT | MMC_MODE_8BIT);

        // Read the EXT_CSD register from the card
        self.mmc_send_ext_csd(&mut ext_csd)?;

        // Determine supported high-speed modes from EXT_CSD
        let avail_type = self.mmc_select_card_type(&ext_csd, host_caps);

        // Select the appropriate high-speed mode supported by both host and card
        let result = if avail_type & EXT_CSD_CARD_TYPE_HS200 != 0 {
            // HS200 mode
            self.mmc_select_hs200(host_caps)
        } else if avail_type & EXT_CSD_CARD_TYPE_HS != 0 {
            // Standard high-speed mode
            self.mmc_select_hs()
        } else {
            Err(MmcHostError::InvalidValue)
        };

        // Apply the result of speed mode selection
        result?;

        // Configure the bus speed according to selected type
        self.mmc_set_bus_speed(avail_type as u32);

        // If HS200 mode was selected, perform tuning procedure
        if self.mmc_card_hs200() {
            let tuning_result = self.mmc_hs200_tuning();

            // Optionally upgrade to HS400 mode if supported and using 8-bit bus
            let bus_width = self.mmc_host_info().bus_width;
            if avail_type & EXT_CSD_CARD_TYPE_HS400 != 0 && bus_width == MMC_BUS_WIDTH_8BIT {
                // self.mmc_select_hs400()?; // Currently not executed
                self.mmc_set_bus_speed(avail_type as u32);
            }

            tuning_result.map_err(|_| MmcHostError::CommandError)
        } else if !self.mmc_card_hs400es() {
            // If not in HS400 Enhanced Stroxbe mode, try to switch bus width
            let width_result = self.mmc_select_bus_width(host_caps)?;
            let err = if width_result > 0 {
                Ok(())
            } else {
                Err(MmcHostError::CommandError)
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

    pub fn mmc_select_hs200(&mut self, host_caps: u32) -> MmcHostResult {
        let ret = self.mmc_select_bus_width(host_caps)?;

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

    fn mmc_select_bus_width(&mut self, host_caps: u32) -> MmcHostResult<i32> {
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

        // Check if host supports 4-bit or 8-bit bus width
        let version = self.card().unwrap().base_info().card_version();
        if version < MMC_VERSION_4 || (host_caps & (MMC_MODE_4BIT | MMC_MODE_8BIT)) == 0 {
            return Ok(0);
        }

        self.mmc_send_ext_csd(&mut ext_csd)?;

        let mut idx = if (host_caps & MMC_MODE_8BIT) != 0 {
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

        Err(MmcHostError::CommandError)
    }

    fn compare_sector_count(&self, ext_csd: &[u8], test_csd: &[u8]) -> bool {
        let sec_cnt_offset = EXT_CSD_SEC_CNT as usize;
        for i in 0..4 {
            if ext_csd[sec_cnt_offset + i] != test_csd[sec_cnt_offset + i] {
                return false;
            }
        }
        true
    }

    // Check if card is write protected
    fn is_write_protected(&self) -> bool {
        let state = self.host_ops().read_reg32(EMMC_PRESENT_STATE);
        (state & EMMC_WRITE_PROTECT) != 0
    }
}
