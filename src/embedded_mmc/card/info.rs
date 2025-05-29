// 临时的内容，后面需要重新优化改进
use super::CardType;

// EMmc Card structure
#[derive(Debug, Clone)]
pub struct MmcCardInfo {
    pub base_info: BaseCardInfo,

    pub state: u32,
    pub block_size: u32,
    pub capacity_blocks: u64,

    pub high_capacity: bool,
    pub version: u32,
    pub dsr: u32,
    pub part_support: u8,
    pub part_attr: u8,
    pub wr_rel_set: u8,
    pub part_config: u8,
    pub dsr_imp: u32,
    pub card_caps: u32,
    pub read_bl_len: u32,
    pub write_bl_len: u32,
    pub erase_grp_size: u32,
    pub hc_wp_grp_size: u64,
    pub capacity: u64,
    pub capacity_user: u64,
    pub capacity_boot: u64,
    pub capacity_rpmb: u64,
    pub capacity_gp: [u64; 4],
    pub enh_user_size: u64,
    pub enh_user_start: u64,
    pub raw_driver_strength: u8,

    // 扩展CSD相关字段
    pub ext_csd_rev: u8,
    pub ext_csd_sectors: u64,
    pub hs_max_dtr: u32,
}

// SdCardInfo structure for SD cards
#[derive(Debug, Clone)]
pub struct SdCardInfo {
    pub base_info: BaseCardInfo,
    
    pub state: u32,
    pub block_size: u32,
    pub capacity_blocks: u64,

    pub version: u32,
    pub dsr: u32,
    pub timing: u32,
    pub clock: u32,
    pub bus_width: u8,
}

#[derive(Debug, Clone)]
pub struct BaseCardInfo {
    pub card_type: CardType,
    pub has_init: bool,
    pub rca: u32,
    pub ocr: u32,
    pub cid: [u32; 4],
    pub csd: [u32; 4],
}
