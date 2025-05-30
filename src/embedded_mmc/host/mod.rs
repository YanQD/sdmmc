// mod emmc;

pub mod sdhci;
pub mod constants;

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;
use log::debug;
use log::info;
use log::trace;
use sdhci::rockship::SdhciHost;

use core::fmt::Debug;
use core::panic;
use alloc::vec::Vec;

use crate::delay_us;
use crate::embedded_mmc::aux::*;
use crate::embedded_mmc::card::CardExt;
use crate::embedded_mmc::card::CardType;
use crate::emmc::constant::*;

use super::card;
use super::card::MmcCard;
use super::commands::DataBuffer;
use super::commands::MmcCommand;

pub enum MmcHostErr {
    CommandError,
    Timeout,
    Unsupported,
}

pub type MmcHostResult<T = ()> = Result<T, MmcHostErr>;

#[derive(Debug)]
pub struct UDevice {
    pub name: String,
    pub compatible: Vec<String>,
}

pub struct MmcHost {
    pub name: String,
    pub card: Option<MmcCard>,
    pub host_ops: SdhciHost,
}

pub trait MmcHostOps: Debug + Send + Sync {
    fn send_cmd(&self, cmd: &MmcCommand, data_buffer: Option<DataBuffer>,) -> MmcHostResult<()>;

    fn card_busy(&self) -> bool;

    fn set_ios(&self) -> MmcHostResult<()>;

    fn get_cd(&self) -> MmcHostResult<bool>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerState {
    // 控制器硬件状态
    PowerOff,           // 控制器未上电
    PowerOn,            // 控制器已上电但未初始化
    Reset,              // 控制器正在复位
    
    // 卡片检测和初始化状态
    Idle,               // 空闲状态，等待卡片插入
    CardDetected,       // 检测到卡片插入
    Identifying,        // 正在识别卡片类型
    Initializing,       // 正在初始化卡片
    
    // 数据传输状态
    Ready,              // 准备就绪，可以执行命令
    CommandActive,      // 正在执行命令
    DataTransfer,       // 正在进行数据传输
    DataTransferRead,   // 正在读取数据
    DataTransferWrite,  // 正在写入数据
    
    // 错误和恢复状态
    Error,              // 发生错误
    Timeout,            // 命令或数据传输超时
    CrcError,           // CRC错误
    Recovering,         // 正在恢复
    
    // 特殊操作状态
    Tuning,             // 正在进行时序调整（用于高速模式）
    Switching,          // 正在切换工作模式（电压、时序等）
    
    // 低功耗状态
    Suspended,          // 挂起状态
    Sleep,              // 卡片进入睡眠模式
}

impl MmcHost {
    fn new(name: String, host_ops: SdhciHost) -> Self {
        MmcHost {
            name,
            card: None,
            host_ops,
        }
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

    fn host_ops(&self) -> &SdhciHost {
        &self.host_ops
    }

    fn host_ops_mut(&mut self) -> &mut SdhciHost {
        &mut self.host_ops
    }

    // Send CMD0 to reset the card
    pub fn mmc_go_idle(&self) -> MmcHostResult {
        let cmd = MmcCommand::new(MMC_GO_IDLE_STATE, 0, MMC_RSP_NONE);
        self.host_ops().send_command(&cmd, None).unwrap();

        delay_us(10000);

        info!("eMMC reset complete");
        Ok(())
    }

    // Send CMD1 to set OCR and check if card is ready
    pub fn mmc_send_op_cond(&mut self, ocr: u32, mut retry: u32) -> MmcHostResult<u32> {
        // First command to get capabilities

        let mut cmd = MmcCommand::new(MMC_SEND_OP_COND, ocr, MMC_RSP_R3);
        self.host_ops().send_command(&cmd, None).unwrap();
        delay_us(10000);

        // Get response and store it
        let mut card_ocr = self.host_ops().get_response().as_r3();

        info!("eMMC first CMD1 response (no args): {:#x}", card_ocr);

        // Calculate arg for next commands
        let ocr_hcs = 0x40000000; // High Capacity Support
        let ocr_busy = 0x80000000;
        let ocr_voltage_mask = 0x007FFF80;
        let ocr_access_mode = 0x60000000;

        let cmd_arg = ocr_hcs
            | (self.host_ops().voltages & (card_ocr & ocr_voltage_mask))
            | (card_ocr & ocr_access_mode);

        // info!("eMMC CMD1 arg for retries: {:#x}", cmd_arg);

        // Now retry with the proper argument until ready or timeout
        let mut ready = false;
        while retry > 0 && !ready {
            cmd = MmcCommand::new(MMC_SEND_OP_COND, cmd_arg, MMC_RSP_R3);
            self.host_ops().send_command(&cmd, None).unwrap();
            let resp = self.host_ops().get_response().as_r3();
            card_ocr = resp;

            info!("CMD1 response raw: {:#x}", self.host_ops().read_reg32(EMMC_RESPONSE));
            info!("eMMC CMD1 response: {:#x}", resp);

            // Update card OCR
            {
                let card = self.card_mut().unwrap();
                card.base_info_mut().set_ocr(resp);

                // Check if card is ready (OCR_BUSY flag set)
                if (resp & ocr_busy) != 0 {
                    ready = true;
                    if (resp & ocr_hcs) != 0 {
                        card.set_card_type(CardType::Mmc);
                        card.set_cardext(CardExt::new(CardType::Mmc));

                        let cardext_mut = card.cardext_mut().unwrap();
                        let mmc_ext = cardext_mut.as_mut_mmc().unwrap();
                        mmc_ext.state |= MMC_STATE_HIGHCAPACITY;
                    }
                }
            }

            if !ready {
                retry -= 1;
                // Delay between retries
                delay_us(1000);
            }
        }

        info!("eMMC initialization status: {}", ready);

        if !ready {
            return Err(MmcHostErr::Unsupported);
        }

        delay_us(1000);

        debug!(
            "Clock control before CMD2: 0x{:x}, stable: {}",
            self.host_ops().read_reg16(EMMC_CLOCK_CONTROL),
            self.host_ops().is_clock_stable()
        );

        Ok(card_ocr)
    }

    // Send CMD2 to get CID
    pub fn mmc_all_send_cid(&mut self) -> MmcHostResult<[u32; 4]> {
        let cmd = MmcCommand::new(MMC_ALL_SEND_CID, 0, MMC_RSP_R2);
        self.host_ops().send_command(&cmd, None).unwrap();
        let response = self.host_ops().get_response();

        // Now borrow card as mutable to update it
        let card = self.card_mut().unwrap();

        card.base_info_mut().set_cid(response.as_r2());

        Ok(card.base_info().cid())
    }

    // Send CMD3 to set RCA for eMMC
    pub fn mmc_set_relative_addr(&self) -> MmcHostResult<> {
        // Get the RCA value before borrowing the card
        let card = self.card().unwrap();
        let rca = card.base_info().rca();

        let cmd = MmcCommand::new(MMC_SET_RELATIVE_ADDR, rca << 16, MMC_RSP_R1);
        self.host_ops().send_command(&cmd, None).unwrap();

        Ok(())
    }

    // Send CMD9 to get CSD
    pub fn mmc_send_csd(&mut self) -> MmcHostResult<[u32; 4]> {
        // Get the RCA value before borrowing the card
        let card = self.card().unwrap();
        let rca = card.base_info().rca();

        let cmd = MmcCommand::new(MMC_SEND_CSD, rca << 16, MMC_RSP_R2);
        self.host_ops().send_command(&cmd, None).unwrap();
        let response = self.host_ops().get_response();

        // Now borrow card as mutable to update it
        let card = self.card_mut().unwrap();
        card.base_info_mut().set_csd(response.as_r2());

        Ok(card.base_info().csd())
    }

    // Send CMD8 to get EXT_CSD
    pub fn mmc_send_ext_csd(&mut self, ext_csd: &mut [u8; 512]) -> MmcHostResult {
        let cmd = MmcCommand::new(MMC_SEND_EXT_CSD, 0, MMC_RSP_R1).with_data(
            MMC_MAX_BLOCK_LEN as u16,
            1,
            true,
        );

        self.host_ops().send_command(&cmd, Some(DataBuffer::Read(ext_csd))).unwrap();

        // debug!("CMD8: {:#x}",self.get_response().as_r1());
        // debug!("EXT_CSD read successfully, rev: {}", ext_csd[EXT_CSD_REV as usize]);

        Ok(())
    }

    // Send CMD6 to switch modes
    fn mmc_switch(
        &self,
        _set: u8,
        index: u32,
        value: u8,
        send_status: bool,
    ) -> MmcHostResult {
        let mut retries = 3;
        let cmd = MmcCommand::new(
            MMC_SWITCH,
            (MMC_SWITCH_MODE_WRITE_BYTE << 24)
                | (index << 16)
                | ((value as u32) << 8),
            MMC_RSP_R1B,
        );

        loop {
            let ret = self.host_ops().send_command(&cmd, None);

            if ret.is_ok() {
                debug!("cmd6 {:#x}", self.host_ops().get_response().as_r1());
                return self.mmc_poll_for_busy(send_status);
            }

            retries -= 1;
            if retries <= 0 {
                debug!("Switch command failed after 3 retries");
                break;
            }
        }

        Err(MmcHostErr::Timeout)
    }

    pub fn mmc_poll_for_busy(&self, send_status: bool) -> MmcHostResult {
        let mut busy = true;
        let mut timeout = 1000;
        let rca = self.card().unwrap().base_info().rca();

        while busy {
            if send_status {
                let cmd = MmcCommand::new(
                    MMC_SEND_STATUS,
                    rca << 16,
                    MMC_RSP_R1,
                );
                self.host_ops().send_command(&cmd, None).unwrap();
                let response = self.host_ops().get_response().as_r1();
                trace!("cmd_d {:#x}", response);

                if response & MMC_STATUS_SWITCH_ERROR != 0 {
                    return Err(MmcHostErr::CommandError);
                }
                busy = (response & MMC_STATUS_CURR_STATE) == MMC_STATE_PRG;
                if !busy {
                    break;
                }
            } else {
                busy = self.mmc_card_busy();
            }

            if timeout == 0 && busy {
                return Err(MmcHostErr::Timeout);
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
            self.host_ops_mut().mmc_set_timing(MMC_TIMING_MMC_HS);
        }

        ret
    }

    pub fn mmc_card_busy(&self) -> bool {
        let present_state = self.host_ops().read_reg32(EMMC_PRESENT_STATE);
        // 检查DATA[0]线是否为0（低电平表示忙）
        !(present_state & EMMC_DATA_0_LVL != 0)
    }

    pub fn mmc_set_dsr(&mut self, dsr: u32) -> MmcHostResult {
        // Set DSR (Driver Stage Register) value
        let cmd = MmcCommand::new(MMC_SET_DSR, dsr, MMC_RSP_NONE);
        self.host_ops().send_command(&cmd, None).unwrap();
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
        let ocr = self.mmc_send_op_cond(ocr, retry)?;

        {
            let card_mut = self.card_mut().unwrap();

            // Set RCA (Relative Card Address)
            card_mut.base_info_mut().set_rca(1);

            // Determine if card is high capacity (SDHC/SDXC/eMMC)
            let high_capacity = (ocr & OCR_HCS) == OCR_HCS;
            card_mut.base_info_mut().set_high_capacity(high_capacity);
        }
    
        // CMD2: Request CID (Card Identification)
        let _cid = self.mmc_all_send_cid()?;

        // CMD3: Set RCA and switch card to "standby" state
        self.mmc_set_relative_addr()?;

        // CMD9: Read CSD (Card-Specific Data) register
        let csd = self.mmc_send_csd()?;

        // Determine card version from CSD if unknown
        let card_version = {
            let card_mut = self.card_mut().unwrap();
            card_mut.base_info().card_version()
        };
        
        if card_version == MMC_VERSION_UNKNOWN {
            let card_mut = self.card_mut().unwrap();
            let csd_version = (card_mut.base_info().csd()[0] >> 26) & 0xf;
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

        // Extract parameters from CSD for frequency, size, and block lengths
        let (freq, mult, dsr_imp, mut read_bl_len, mut write_bl_len, csize, cmult) = {
            let card_mut = self.card_mut().unwrap();
            let freq = FBASE[(csd[0] & 0x7) as usize];
            let mult = MULTIPLIERS[((csd[0] >> 3) & 0xf) as usize];
            let dsr_imp = ((csd[1] >> 12) & 0x1) != 0;
            let read_bl_len = 1 << ((csd[1] >> 16) & 0xf);
            let card_type = card_mut.card_type();
            let write_bl_len = if card_type == CardType::Mmc {
                1 << ((csd[3] >> 22) & 0xf)
            } else if card_type == CardType::SdV1 || card_type == CardType::SdV2 {
                read_bl_len
            } else {
                panic!("Unsupported card type for write block length: {:?}", card_type);
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
            card_mut.base_info_mut().set_dsr_imp(dsr_imp);
            (freq, mult, dsr_imp, read_bl_len, write_bl_len, csize, cmult)
        };

        // Calculate user capacity
        let _tran_speed = freq * mult as usize;
        let mut capacity_user = (csize as u64 + 1) << (cmult as u64 + 2);
        capacity_user *= read_bl_len as u64;
        let capacity_boot = 0;
        let capacity_rpmb = 0;

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

        // CMD4: Set DSR if required by card
        let (dsr_needed, dsr) = {
            let dsr = card_mut.base_info().dsr();
            ((dsr_imp as u8 != 0) && 0xffffffff != dsr, dsr)
        };

        // Set initial erase group size and partition config
        card_mut.base_info_mut().set_erase_grp_size(1);
        card_mut.base_info_mut().set_part_config(MMCPART_NOAVAILABLE);

        if dsr_needed {
            let dsr_value = {
                (dsr & 0xffff) << 16
            };
            self.mmc_set_dsr(dsr_value)?;
        }

        // CMD7: Select the card
        let cmd7 = {
            let card_mut = self.card_mut().unwrap();
            let rca = card_mut.base_info_mut().rca();
            MmcCommand::new(MMC_SELECT_CARD, rca << 16, MMC_RSP_R1)
        };

        self.host_ops().send_command(&cmd7, None).unwrap();
        debug!("cmd7: {:#x}", self.host_ops().get_response().as_r1());

        // For eMMC 4.0+, configure high-speed, EXT_CSD and partitions
        let is_version_4_plus = {
            let card = self.card().unwrap();
            let card_type = card.card_type();
            card_type == CardType::Mmc && card.base_info().card_version() >= MMC_VERSION_4
        };

        if is_version_4_plus {
            self.mmc_select_hs()?; // Switch to high speed
            self.host_ops_mut().mmc_set_clock(MMC_HIGH_52_MAX_DTR); // Set high-speed clock

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

            // // Extract capacity and version
            // if ext_csd[EXT_CSD_REV as usize] >= 2 {
            //     let mut capacity: u64 = ext_csd[EXT_CSD_SEC_CNT as usize] as u64
            //         | (ext_csd[EXT_CSD_SEC_CNT as usize + 1] as u64) << 8
            //         | (ext_csd[EXT_CSD_SEC_CNT as usize + 2] as u64) << 16
            //         | (ext_csd[EXT_CSD_SEC_CNT as usize + 3] as u64) << 24;
            //     capacity *= MMC_MAX_BLOCK_LEN as u64;
            //     if (capacity >> 20) > 2 * 1024 {
            //         self.set_capacity_user(capacity).unwrap();
            //     }

            //     let card = self.card_mut().unwrap();
            //     match ext_csd[EXT_CSD_REV as usize] {
            //         1 => card.base_info_mut().set_card_version(MMC_VERSION_4_1),
            //         2 => card.base_info_mut().set_card_version(MMC_VERSION_4_2),
            //         3 => card.base_info_mut().set_card_version(MMC_VERSION_4_3),
            //         5 => card.base_info_mut().set_card_version(MMC_VERSION_4_41),
            //         6 => card.base_info_mut().set_card_version(MMC_VERSION_4_5),
            //         7 => card.base_info_mut().set_card_version(MMC_VERSION_5_0),
            //         8 => card.base_info_mut().set_card_version(MMC_VERSION_5_1),
            //         _ => panic!("Unknown EXT_CSD revision"),
            //     }
            // }

            // // Parse partition configuration info
            // let part_completed = (ext_csd[EXT_CSD_PARTITION_SETTING as usize] as u32
            //     & EXT_CSD_PARTITION_SETTING_COMPLETED)
            //     != 0;
            // self.set_part_support(ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize])
            //     .unwrap();

            // if (ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize] as u32 & PART_SUPPORT != 0)
            //     || ext_csd[EXT_CSD_BOOT_MULT as usize] != 0
            // {
            //     self.set_part_config(ext_csd[EXT_CSD_PART_CONF as usize])
            //         .unwrap();
            // }

            // // Save enhanced partition attributes
            // if part_completed
            //     && (ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize] as u32 & ENHNCD_SUPPORT != 0)
            // {
            //     let part_attr = ext_csd[EXT_CSD_PARTITIONS_ATTRIBUTE as usize];
            //     self.set_part_attr(part_attr).unwrap();
            // }

            // // Check secure erase support
            // if ext_csd[EXT_CSD_SEC_FEATURE_SUPPORT as usize] as u32 & EXT_CSD_SEC_GB_CL_EN != 0 {
            //     let _mmc_can_trim = 1;
            // }

            // // Calculate boot and RPMB sizes
            // let capacity_boot = (ext_csd[EXT_CSD_BOOT_MULT as usize] as u64) << 17;
            // self.set_capacity_boot(capacity_boot).unwrap();
            // let capacity_rpmb = (ext_csd[EXT_CSD_RPMB_MULT as usize] as u64) << 17;
            // self.set_capacity_rpmb(capacity_rpmb).unwrap();
            // debug!("Boot partition size: {:#x}", capacity_boot);
            // debug!("RPMB partition size: {:#x}", capacity_rpmb);

            // // Calculate general purpose partition sizes
            // let mut has_parts = false;
            // for i in 0..4 {
            //     let idx = EXT_CSD_GP_SIZE_MULT as usize + i * 3;
            //     let mult = ((ext_csd[idx + 2] as u32) << 16)
            //         + ((ext_csd[idx + 1] as u32) << 8)
            //         + (ext_csd[idx] as u32);
            //     if mult != 0 {
            //         has_parts = true;
            //     }
            //     if !part_completed {
            //         continue;
            //     }
            //     capacity_gp[i] = mult as u64;
            //     capacity_gp[i] *= ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u64;
            //     capacity_gp[i] *= ext_csd[EXT_CSD_HC_WP_GRP_SIZE as usize] as u64;
            //     capacity_gp[i] <<= 19;
            //     self.set_capacity_gp(capacity_gp).unwrap();
            // }
            // debug!("GP partition sizes: {:?}", capacity_gp);

            // // Calculate enhanced user data size and start
            // if part_completed {
            //     let mut enh_user_size = ((ext_csd[EXT_CSD_ENH_SIZE_MULT as usize + 2] as u64)
            //         << 16)
            //         + ((ext_csd[EXT_CSD_ENH_SIZE_MULT as usize + 1] as u64) << 8)
            //         + (ext_csd[EXT_CSD_ENH_SIZE_MULT as usize] as u64);
            //     enh_user_size *= ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u64;
            //     enh_user_size *= ext_csd[EXT_CSD_HC_WP_GRP_SIZE as usize] as u64;
            //     enh_user_size <<= 19;
            //     self.set_enh_user_size(enh_user_size).unwrap();

            //     let mut enh_user_start = ((ext_csd[EXT_CSD_ENH_START_ADDR as usize + 3] as u64)
            //         << 24)
            //         + ((ext_csd[EXT_CSD_ENH_START_ADDR as usize + 2] as u64) << 16)
            //         + ((ext_csd[EXT_CSD_ENH_START_ADDR as usize + 1] as u64) << 8)
            //         + (ext_csd[EXT_CSD_ENH_START_ADDR as usize] as u64);
            //     if high_capacity {
            //         enh_user_start <<= 9;
            //     }
            //     self.set_enh_user_start(enh_user_start).unwrap();
            // }

            // // If partitions are configured, enable ERASE_GRP_DEF
            // if part_completed {
            //     has_parts = true;
            // }

            // if (ext_csd[EXT_CSD_PARTITIONING_SUPPORT as usize] as u32 & PART_SUPPORT != 0)
            //     && (ext_csd[EXT_CSD_PARTITIONS_ATTRIBUTE as usize] as u32 & PART_ENH_ATTRIB != 0)
            // {
            //     has_parts = true;
            // }

            // if has_parts {
            //     let err = self.mmc_switch(EXT_CSD_CMD_SET_NORMAL, EXT_CSD_ERASE_GROUP_DEF, 1, true);
            //     if err.is_err() {
            //         return Err(MmcHostErr::CommandError);
            //     } else {
            //         ext_csd[EXT_CSD_ERASE_GROUP_DEF as usize] = 1;
            //     }
            // }

            // // Calculate erase group size
            // if ext_csd[EXT_CSD_ERASE_GROUP_DEF as usize] & 0x01 != 0 {
            //     self.set_erase_grp_size(
            //         (ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u32) * 1024,
            //     )
            //     .unwrap();

            //     if high_capacity && part_completed {
            //         let capacity = (ext_csd[EXT_CSD_SEC_CNT as usize] as u64)
            //             | ((ext_csd[EXT_CSD_SEC_CNT as usize + 1] as u64) << 8)
            //             | ((ext_csd[EXT_CSD_SEC_CNT as usize + 2] as u64) << 16)
            //             | ((ext_csd[EXT_CSD_SEC_CNT as usize + 3] as u64) << 24);
            //         self.set_capacity_user(capacity * (MMC_MAX_BLOCK_LEN as u64))
            //             .unwrap();
            //     }
            // } else {
            //     let erase_gsz = (csd[2] & 0x00007c00) >> 10;
            //     let erase_gmul = (csd[2] & 0x000003e0) >> 5;
            //     self.set_erase_grp_size((erase_gsz + 1) * (erase_gmul + 1))
            //         .unwrap();
            // }

            // // Set high-capacity write-protect group size
            // let hc_wp_grp_size = 1024
            //     * (ext_csd[EXT_CSD_HC_ERASE_GRP_SIZE as usize] as u64)
            //     * (ext_csd[EXT_CSD_HC_WP_GRP_SIZE as usize] as u64);
            // self.set_hc_wp_grp_size(hc_wp_grp_size).unwrap();

            // // Set write reliability and drive strength
            // self.set_wr_rel_set(ext_csd[EXT_CSD_WR_REL_SET as usize])
            //     .unwrap();
            // self.set_raw_driver_strength(ext_csd[EXT_CSD_DRIVER_STRENGTH as usize])
            //     .unwrap();
        }

        // // Final initialization steps
        // self.mmc_set_capacity(0)?;
        // self.mmc_change_freq()?;
        self.card_mut().unwrap().set_initialized(true);

        Ok(())
    }

    pub fn init(&mut self) -> MmcHostResult {
        info!("eMMC host initialization started");

        self.host_ops_mut().init_host().unwrap();


        info!("eMMC host initialization complete");
        Ok(())
    }
}