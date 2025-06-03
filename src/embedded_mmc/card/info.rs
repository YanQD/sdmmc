use crate::impl_accessors;

// 临时的内容，后面需要重新优化改进
use super::CardType;

// EMmc Card structure
#[derive(Debug, Clone)]
pub struct MmcCardExt {
    pub state: u32,
    pub capacity_blocks: u64,

    pub part_support: u8,
    pub part_attr: u8,
    pub wr_rel_set: u8,
    pub hc_wp_grp_size: u64,
    pub capacity_user: u64,
    pub capacity_boot: u64,
    pub capacity_rpmb: u64,
    pub capacity_gp: [u64; 4],
    pub enh_user_size: u64,
    pub enh_user_start: u64,
    pub raw_driver_strength: u8,

    pub ext_csd_rev: u8,
    pub ext_csd_sectors: u64,
    pub hs_max_dtr: u32,

    pub emmc_esr: EmmcStatusRegister,
}

impl MmcCardExt {
    pub fn new() -> Self {
        MmcCardExt {
            state: 0,
            capacity_blocks: 0,
            part_support: 0,
            part_attr: 0,
            wr_rel_set: 0,
            hc_wp_grp_size: 0,
            capacity_user: 0,
            capacity_boot: 0,
            capacity_rpmb: 0,
            capacity_gp: [0; 4],
            enh_user_size: 0,
            enh_user_start: 0,
            raw_driver_strength: 0,

            ext_csd_rev: 0,
            ext_csd_sectors: 0,
            hs_max_dtr: 0,

            emmc_esr: EmmcStatusRegister::default(),
        }
    }
}

// SdCardInfo structure for SD cards
#[derive(Debug, Clone)]
pub struct SdCardExt {
    pub state: u32,
    pub capacity_blocks: u64,
    pub dsr: u32,
    pub sd_ssr: Option<SdStatusRegister>,
}

impl SdCardExt {
    pub fn new() -> Self {
        SdCardExt {
            state: 0,
            capacity_blocks: 0,
            dsr: 0,
            sd_ssr: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BaseCardInfo {
    rca: u32,
    ocr: u32,
    cid: [u32; 4],
    csd: [u32; 4],
    high_capacity: bool,
    card_version: u32,
    read_bl_len: u32,
    write_bl_len: u32,
    dsr_imp: bool,
    dsr: u32,
    capacity: u64,
    erase_grp_size: u32,
    part_config: u8,
    block_size: u32,
    card_caps: u32,
}

impl BaseCardInfo {
    pub fn new() -> Self {
        BaseCardInfo {
            rca: 0,
            ocr: 0,
            cid: [0; 4],
            csd: [0; 4],
            high_capacity: false,
            card_version: 0,
            read_bl_len: 0,
            write_bl_len: 0,
            dsr_imp: false,
            dsr: 0,
            capacity: 0,
            erase_grp_size: 0,
            part_config: 0,
            block_size: 512, // Default block size
            card_caps: 0,
        }
    }
}

impl_accessors!(
    BaseCardInfo,
    rca: u32,
    ocr: u32,
    cid: [u32; 4],
    csd: [u32; 4],
    high_capacity: bool,
    card_version: u32,
    read_bl_len: u32,
    write_bl_len: u32,
    dsr_imp: bool,
    dsr: u32,
    capacity: u64,
    erase_grp_size: u32,
    part_config: u8,
    block_size: u32,
    card_caps: u32
);

#[derive(Debug, Clone, Default)]
pub struct EmmcStatusRegister {
    pub mmc_can_trim: bool,
}

impl EmmcStatusRegister {
    pub fn supports_trim(&self) -> bool {
        self.mmc_can_trim
    }
}

#[derive(Debug, Clone, Default)]
pub struct SdStatusRegister {
    /// Allocation Unit size in sectors (AU)
    pub au: u32,

    /// Erase timeout in milliseconds
    pub erase_timeout: u32,

    /// Erase offset in milliseconds  
    pub erase_offset: u32,
}

impl SdStatusRegister {
    pub fn au_size_bytes(&self) -> u64 {
        self.au as u64 * 512
    }

    pub fn preferred_write_alignment(&self) -> u32 {
        if self.au > 0 { self.au * 512 } else { 4096 }
    }

    pub fn actual_erase_time(&self) -> u32 {
        self.erase_timeout + self.erase_offset
    }
}
