// mod emmc;

pub mod sdhci;
pub mod constants;

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;
use sdhci::rockship::SdhciHost;

use core::fmt::Debug;
use alloc::vec::Vec;

use super::card::MmcCard;
use super::commands::DataBuffer;
use super::commands::MmcCommand;

pub enum MmcHostErr {

}

pub type MmcHostResult<T> = Result<T, MmcHostErr>;

#[derive(Debug)]
pub struct UDevice {
    pub name: String,
    pub compatible: Vec<String>,
}

pub struct MmcHost {
    pub name: String,
    pub card: MmcCard,
    pub ops: SdhciHost,
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
