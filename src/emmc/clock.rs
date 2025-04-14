use core::fmt;
use core::ptr::{self, read_volatile, write_volatile};
use log::debug;

/// RK3568 eMMC时钟源选择常量
const CCLK_EMMC_SEL_24M: u32 = 0;  // OSC (24MHz)
const CCLK_EMMC_SEL_200M: u32 = 1; // 200 MHz
const CCLK_EMMC_SEL_150M: u32 = 2; // 150 MHz
const CCLK_EMMC_SEL_100M: u32 = 3; // 100 MHz
const CCLK_EMMC_SEL_50M: u32 = 4;  // 50 MHz
const CCLK_EMMC_SEL_375K: u32 = 5; // 375 KHz

/// eMMC 总线时钟(BCLK)选择常量
const BCLK_EMMC_SEL_200M: u32 = 0; // 200 MHz
const BCLK_EMMC_SEL_150M: u32 = 1; // 150 MHz
const BCLK_EMMC_SEL_125M: u32 = 2; // 125 MHz

/// eMMC 总线时钟选择掩码和偏移
const BCLK_EMMC_SEL_MASK: u32 = 0x3 << 8;
const BCLK_EMMC_SEL_SHIFT: u32 = 8;

/// 频率常量
const MHZ: u64 = 1_000_000;
const KHZ: u64 = 1_000;
const OSC_HZ: u64 = 24 * MHZ;      // 24 MHz

/// eMMC时钟选择掩码和偏移
const CCLK_EMMC_SEL_MASK: u32 = 0x7 << 12;
const CCLK_EMMC_SEL_SHIFT: u32 = 12;

/// 错误类型
#[derive(Debug, Clone, Copy)]
pub enum RK3568Error {
    InvalidClockRate,
    RegisterOperationFailed,
}

/// RK3568 时钟控制单元寄存器结构
#[repr(C)]
pub struct RK3568Cru {
    pll: [u32; 24],             // PLL 寄存器
    mode_con00: u32,            // 模式控制寄存器
    misc_con: [u32; 3],         // 杂项控制寄存器
    glb_cnt_th: u32,            // 全局计数阈值
    glb_rst_regs: [u32; 3],     // 全局复位寄存器组
    reserved0: [u32; 7],        // 保留
    clksel_con: [u32; 85],      // 时钟选择寄存器
    reserved1: [u32; 43],       // 保留
    clkgate_con: [u32; 36],     // 时钟门控寄存器
    reserved2: [u32; 28],       // 保留
    softrst_con: [u32; 30],     // 软复位寄存器
    reserved3: [u32; 2],        // 保留
    ssgtbl: [u32; 32],          // SSG表寄存器
    reserved4: [u32; 32],       // 保留
    sdmmc0_con: [u32; 2],       // SDMMC0控制寄存器
    sdmmc1_con: [u32; 2],       // SDMMC1控制寄存器
    sdmmc2_con: [u32; 2],       // SDMMC2控制寄存器
    emmc_con: [u32; 2],         // eMMC控制寄存器
}

/// RK3568 时钟驱动
pub struct RK3568ClkPriv {
    cru: *mut RK3568Cru,
}

impl fmt::Display for RK3568Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RK3568Error::InvalidClockRate => write!(f, "Invalid clock rate"),
            RK3568Error::RegisterOperationFailed => write!(f, "Register operation failed"),
        }
    }
}

impl RK3568ClkPriv {
    pub unsafe fn new(cru_ptr: *mut RK3568Cru) -> Self {
        // cru的基地址
        Self {
            cru: cru_ptr,
        }
    }

    /// 获取当前eMMC时钟频率
    pub fn emmc_get_clk(&self) -> Result<u64, RK3568Error> {
        // 安全地读取寄存器
        let con = unsafe { read_volatile(&(*self.cru).clksel_con[28]) };
        
        // 提取时钟选择位
        let sel = (con & CCLK_EMMC_SEL_MASK) >> CCLK_EMMC_SEL_SHIFT;
        
        // 根据选择返回对应频率
        match sel {
            CCLK_EMMC_SEL_200M => Ok(200 * MHZ),
            CCLK_EMMC_SEL_150M => Ok(150 * MHZ),
            CCLK_EMMC_SEL_100M => Ok(100 * MHZ),
            CCLK_EMMC_SEL_50M => Ok(50 * MHZ),
            CCLK_EMMC_SEL_375K => Ok(375 * KHZ),
            CCLK_EMMC_SEL_24M => Ok(OSC_HZ),
            _ => Err(RK3568Error::InvalidClockRate),
        }
    }
    
    /// 设置eMMC时钟频率
    pub fn emmc_set_clk(&mut self, rate: u64) -> Result<u64, RK3568Error> {
        debug!("cru = {:p}, rate = {}", self.cru, rate);
        
        // 根据请求的频率选择对应的时钟源
        let src_clk = match rate {
            OSC_HZ => CCLK_EMMC_SEL_24M,
            r if r == 52 * MHZ || r == 50 * MHZ => CCLK_EMMC_SEL_50M,
            r if r == 100 * MHZ => CCLK_EMMC_SEL_100M,
            r if r == 150 * MHZ => CCLK_EMMC_SEL_150M,
            r if r == 200 * MHZ => CCLK_EMMC_SEL_200M,
            r if r == 400 * KHZ || r == 375 * KHZ => CCLK_EMMC_SEL_375K,
            _ => return Err(RK3568Error::InvalidClockRate),
        };
        
        unsafe {
            let addr = &mut (*self.cru).clksel_con[28];

            self.rk_clrsetreg(
                addr,
                CCLK_EMMC_SEL_MASK,
                src_clk << CCLK_EMMC_SEL_SHIFT
            );
        }
        
        // 返回实际设置的频率
        self.emmc_get_clk()
    }

    /// 获取当前 eMMC 总线时钟频率
    pub fn emmc_get_bclk(&self) -> Result<u64, RK3568Error> {
        // 安全地读取寄存器
        let con = unsafe { read_volatile(&(*self.cru).clksel_con[28]) };
        
        // 提取时钟选择位
        let sel = (con & BCLK_EMMC_SEL_MASK) >> BCLK_EMMC_SEL_SHIFT;
        
        // 根据选择返回对应频率
        match sel {
            BCLK_EMMC_SEL_200M => Ok(200 * MHZ),
            BCLK_EMMC_SEL_150M => Ok(150 * MHZ),
            BCLK_EMMC_SEL_125M => Ok(125 * MHZ),
            _ => Err(RK3568Error::InvalidClockRate),
        }
    }
    
    /// 设置 eMMC 总线时钟频率
    pub fn emmc_set_bclk(&mut self, rate: u64) -> Result<u64, RK3568Error> {
        // 根据请求的频率选择对应的时钟源
        let src_clk = match rate {
            r if r == 200 * MHZ => BCLK_EMMC_SEL_200M,
            r if r == 150 * MHZ => BCLK_EMMC_SEL_150M,
            r if r == 125 * MHZ => BCLK_EMMC_SEL_125M,
            _ => return Err(RK3568Error::InvalidClockRate),
        };
        
        unsafe {
            // 读取-修改-写入操作
            let addr = &mut (*self.cru).clksel_con[28];

            self.rk_clrsetreg(
                addr,
                BCLK_EMMC_SEL_MASK,
                src_clk << BCLK_EMMC_SEL_SHIFT
            );
        }
        
        // 返回实际设置的频率
        self.emmc_get_bclk()
    }
    
    /// 清除并设置寄存器的特定位
    pub fn rk_clrsetreg(&self, addr: *mut u32, clr: u32, set: u32) {
        let val = ((clr | set) << 16) | set;
        unsafe { write_volatile(addr, val) };
    }
}

/// MMC相关常量及实现
pub mod mmc {
    use super::*;
    
    /// 相位调整相关常量
    const ROCKCHIP_MMC_DELAY_SEL: u32 = 0x1;
    const ROCKCHIP_MMC_DEGREE_MASK: u32 = 0x3;
    const ROCKCHIP_MMC_DELAYNUM_OFFSET: u32 = 2;
    const ROCKCHIP_MMC_DELAYNUM_MASK: u32 = 0xff << ROCKCHIP_MMC_DELAYNUM_OFFSET;
    const ROCKCHIP_MMC_DELAY_ELEMENT_PSEC: u32 = 100;

    /// MMC采样相位时钟ID
    #[derive(Debug, Clone, Copy)]
    pub enum RK3568MmcClockId {
        SclkEmmcSample,
        SclkSdmmc0Sample,
        SclkSdmmc1Sample,
        SclkSdmmc2Sample,
    }

    /// 时钟结构
    pub struct Clock {
        pub id: RK3568MmcClockId,
        pub rate: u64,
    }

    impl RK3568ClkPriv {
        /// 获取MMC时钟相位(以度为单位)
        pub fn mmc_get_phase(&self, clk: &Clock) -> Result<u16, RK3568Error> {
            // 获取时钟频率
            let rate = clk.rate;
            if rate == 0 {
                return Err(RK3568Error::InvalidClockRate);
            }
            
            // 根据时钟ID读取相应的控制寄存器
            let raw_value = unsafe {
                match clk.id {
                    RK3568MmcClockId::SclkEmmcSample => 
                        read_volatile(&(*self.cru).emmc_con[1]),
                    RK3568MmcClockId::SclkSdmmc0Sample => 
                        read_volatile(&(*self.cru).sdmmc0_con[1]),
                    RK3568MmcClockId::SclkSdmmc1Sample => 
                        read_volatile(&(*self.cru).sdmmc1_con[1]),
                    RK3568MmcClockId::SclkSdmmc2Sample => 
                        read_volatile(&(*self.cru).sdmmc2_con[1]),
                }
            };
            
            let raw_value = raw_value >> 1;
            
            // 计算粗调相位(90度增量)
            let mut degrees = (raw_value & ROCKCHIP_MMC_DEGREE_MASK) * 90;
            
            // 检查是否启用了细调
            if (raw_value & ROCKCHIP_MMC_DELAY_SEL) != 0 {
                // 计算延迟元素带来的额外度数
                let factor = (ROCKCHIP_MMC_DELAY_ELEMENT_PSEC / 10) as u64 *
                              36 * (rate / 1_000_000);
                
                let delay_num = (raw_value & ROCKCHIP_MMC_DELAYNUM_MASK) >> ROCKCHIP_MMC_DELAYNUM_OFFSET;
                
                // 添加细调相位
                degrees += div_round_closest((delay_num as u64 * factor) as u32, 10000);
            }
            
            // 返回总相位(限制在0-359度)
            Ok(degrees as u16 % 360)
        }
        
        /// 设置MMC时钟相位
        pub fn mmc_set_phase(&mut self, clk: &Clock, degrees: u32) -> Result<(), RK3568Error> {
            let rate = clk.rate;
            if rate == 0 {
                return Err(RK3568Error::InvalidClockRate);
            }
            
            // 将请求的相位分解为90度步进和余数
            let nineties = degrees / 90;
            let remainder = degrees % 90;
            
            // 将余数转换为延迟元素数量
            let mut delay = 10_000_000; // PSECS_PER_SEC / 10000 / 10
            delay *= remainder;
            delay = div_round_closest(
                delay,
                (rate / 1000) * 36 * (ROCKCHIP_MMC_DELAY_ELEMENT_PSEC / 10) as u64
            ) as u32;
            
            // 限制延迟元素数量到最大允许值
            let delay_num = core::cmp::min(delay, 255) as u8;
            
            // 构建寄存器值
            let mut raw_value = if delay_num > 0 { ROCKCHIP_MMC_DELAY_SEL } else { 0 };
            raw_value |= (delay_num as u32) << ROCKCHIP_MMC_DELAYNUM_OFFSET;
            raw_value |= nineties;
            
            // 左移1位，以匹配寄存器布局
            raw_value <<= 1;
            
            // 向寄存器写入新值 (0xffff0000用于保留高16位)
            unsafe {
                let addr = match clk.id {
                    RK3568MmcClockId::SclkEmmcSample => 
                        &mut (*self.cru).emmc_con[1],
                    RK3568MmcClockId::SclkSdmmc0Sample => 
                        &mut (*self.cru).sdmmc0_con[1],
                    RK3568MmcClockId::SclkSdmmc1Sample => 
                        &mut (*self.cru).sdmmc1_con[1],
                    RK3568MmcClockId::SclkSdmmc2Sample => 
                        &mut (*self.cru).sdmmc2_con[1],
                };
                write_volatile(addr, raw_value | 0xffff0000);
            }
            
            
            if let Ok(actual_degrees) = self.mmc_get_phase(clk) {
                debug!(
                    "mmc set_phase({}) delay_nums={} reg={:#x} actual_degrees={}", 
                    degrees, delay_num, raw_value, actual_degrees
                );
            }
            
            Ok(())
        }
    }
}

/// 辅助函数: 四舍五入的除法
fn div_round_closest(dividend: u32, divisor: u64) -> u32 {
    ((dividend as u64 + divisor / 2) / divisor) as u32
}