use core::fmt;
use core::ptr::{read_volatile, write_volatile};
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

/// 复位相关常量
const SOFTRST_MASK: u32 = 0x1;

/// 时钟门控掩码
const CLKGATE_MASK: u32 = 0x1;

/// RK3568 外设ID定义
#[derive(Debug, Clone, Copy)]
pub enum RK3568PeripheralId {
    Emmc,
    Sdmmc0,
    Sdmmc1,
    Sdmmc2,
}

/// 错误类型
#[derive(Debug, Clone, Copy)]
pub enum RK3568Error {
    InvalidClockRate,
    RegisterOperationFailed,
    InvalidPeripheralId,
    ResetTimeout,
}

/// RK3568 时钟控制单元寄存器结构
#[repr(C)]
pub struct RK3568Cru {
    pll: [u32; 24],             // PLL 寄存器
    mode_con00: u32,            // 模式控制寄存器
    misc_con: [u32; 3],         // 杂项控制寄存器
    glb_cnt_th: u32,            // 全局计数阈值
    glb_srst_fst: u32,          // 全局软复位
    glb_srsr_snd: u32,          // 全局软复位
    glb_rst_con: u32,           // 全局软复位阈值
    glb_rst_st: u32,            // 全局软复位状态
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
pub struct RK3568ClkPri {
    cru: *mut RK3568Cru,
}

impl fmt::Display for RK3568Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RK3568Error::InvalidClockRate => write!(f, "Invalid clock rate"),
            RK3568Error::RegisterOperationFailed => write!(f, "Register operation failed"),
            RK3568Error::InvalidPeripheralId => write!(f, "Invalid peripheral ID"),
            RK3568Error::ResetTimeout => write!(f, "Reset operation timed out"),
        }
    }
}

/// 外设复位配置表
struct PeripheralResetConfig {
    softrst_con_idx: usize,  // softrst_con 寄存器数组索引
    softrst_bit: u32,        // 软复位位位置
    clkgate_con_idx: usize,  // clkgate_con 寄存器数组索引
    clkgate_bit: u32,        // 时钟门控位位置
}

impl RK3568ClkPri {
    pub unsafe fn new(cru_ptr: *mut RK3568Cru) -> Self {
        // cru的基地址
        Self {
            cru: cru_ptr,
        }
    }

    /// 获取外设复位配置
    fn get_peripheral_reset_config(&self, peripheral: RK3568PeripheralId) -> Result<PeripheralResetConfig, RK3568Error> {
        // 返回每个外设的复位寄存器配置
        match peripheral {
            RK3568PeripheralId::Emmc => Ok(PeripheralResetConfig {
                softrst_con_idx: 12,
                softrst_bit: 14,
                clkgate_con_idx: 9,
                clkgate_bit: 7,
            }),
            _ => Err(RK3568Error::InvalidPeripheralId),
        }
    }

    /// 复位单个外设
    pub fn reset_peripheral(&mut self, peripheral: RK3568PeripheralId) -> Result<(), RK3568Error> {
        // 获取外设复位配置
        let config = self.get_peripheral_reset_config(peripheral)?;
        
        // 安全地访问CRU寄存器
        unsafe {
            // 1. 首先禁用时钟
            let clkgate_addr = &mut (*self.cru).clkgate_con[config.clkgate_con_idx];
            self.rk_clrsetreg(
                clkgate_addr,
                CLKGATE_MASK << config.clkgate_bit,
                CLKGATE_MASK << config.clkgate_bit  // 1 = 禁用时钟
            );
            
            // 2. 应用软复位 (1 = 复位有效)
            let softrst_addr = &mut (*self.cru).softrst_con[config.softrst_con_idx];
            self.rk_clrsetreg(
                softrst_addr,
                SOFTRST_MASK << config.softrst_bit,
                SOFTRST_MASK << config.softrst_bit
            );
            
            // 3. 短暂延迟，确保复位完成
            for _ in 0..100 {
                core::hint::spin_loop();
            }
            
            // 4. 释放软复位 (0 = 复位取消)
            self.rk_clrsetreg(
                softrst_addr,
                SOFTRST_MASK << config.softrst_bit,
                0
            );
            
            // 5. 重新使能时钟
            self.rk_clrsetreg(
                clkgate_addr,
                CLKGATE_MASK << config.clkgate_bit,
                0  // 0 = 使能时钟
            );
        }
        
        debug!("Reset peripheral {:?} completed", peripheral);
        Ok(())
    }

    /// 全局复位系统时钟
    pub fn reset_clock_system(&mut self) -> Result<(), RK3568Error> {
        unsafe {
            // 1. 设置全局软复位控制值 (可能需要特定值/模式)
            write_volatile(&mut (*self.cru).glb_rst_con, 0x1);
            
            // 2. 触发第一阶段复位
            write_volatile(&mut (*self.cru).glb_srst_fst, 0x1);
            
            // 3. 等待复位状态变化 (可能需要检查特定位)
            let mut timeout = 1000;
            while (read_volatile(&(*self.cru).glb_rst_st) & 0x1) == 0 {
                if timeout == 0 {
                    return Err(RK3568Error::ResetTimeout);
                }
                timeout -= 1;
                core::hint::spin_loop();
            }
            
            // 4. 触发第二阶段复位
            write_volatile(&mut (*self.cru).glb_srsr_snd, 0x1);

            debug!("Global clock system reset completed");
        }
        
        Ok(())
    }

    /// 复位所有外设时钟
    pub fn reset_all_peripherals(&mut self) -> Result<(), RK3568Error> {
        // 列出要复位的所有外设
        let peripherals = [
            RK3568PeripheralId::Emmc,
            RK3568PeripheralId::Sdmmc0,
            RK3568PeripheralId::Sdmmc1,
            RK3568PeripheralId::Sdmmc2,
        ];
        
        // 逐个复位外设
        for &peripheral in peripherals.iter() {
            if let Err(e) = self.reset_peripheral(peripheral) {
                debug!("Failed to reset peripheral {:?}: {}", peripheral, e);
                // 继续复位其他外设，不中断流程
            }
        }
        
        debug!("All peripherals reset completed");
        Ok(())
    }

    /// 复位特定类型的所有外设
    pub fn reset_peripheral_group(&mut self, group_type: PeripheralGroup) -> Result<(), RK3568Error> {
        let peripherals = match group_type {
            PeripheralGroup::Storage => [
                RK3568PeripheralId::Emmc,
                RK3568PeripheralId::Sdmmc0,
                RK3568PeripheralId::Sdmmc1,
                RK3568PeripheralId::Sdmmc2,
            ].as_slice(),
            _ => {
                return Err(RK3568Error::InvalidPeripheralId);
            }
        };
        
        // 逐个复位外设
        for &peripheral in peripherals {
            if let Err(e) = self.reset_peripheral(peripheral) {
                debug!("Failed to reset peripheral {:?}: {}", peripheral, e);
                // 继续复位其他外设，不中断流程
            }
        }
        
        debug!("Peripheral group {:?} reset completed", group_type);
        Ok(())
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
    /// 未使用
    pub fn emmc_get_bclk(&self) -> Result<u64, RK3568Error> {
        // 安全地读取寄存器
        let con = unsafe { read_volatile(&(*self.cru).clksel_con[28]) };
        
        // 提取时钟选择位
        let sel = (con & BCLK_EMMC_SEL_MASK) >> BCLK_EMMC_SEL_SHIFT;

        debug!("emmc_get_bclk con = {:#x} sel = {}", con, sel);
        
        // 根据选择返回对应频率
        match sel {
            BCLK_EMMC_SEL_200M => Ok(200 * MHZ),
            BCLK_EMMC_SEL_150M => Ok(150 * MHZ),
            BCLK_EMMC_SEL_125M => Ok(125 * MHZ),
            _ => Err(RK3568Error::InvalidClockRate),
        }
    }
    
    /// 设置 eMMC 总线时钟频率
    /// 未使用
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

/// 外设分组枚举，用于批量复位操作
#[derive(Debug, Clone, Copy)]
pub enum PeripheralGroup {
    Storage,        // eMMC和SD卡控制器
    Communication,  // I2C, SPI等通信接口
    Network,        // 以太网控制器
    UsbController,  // USB控制器
    SerialPort,     // UART串口
}

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

impl RK3568ClkPri {
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

/// 辅助函数: 四舍五入的除法
fn div_round_closest(dividend: u32, divisor: u64) -> u32 {
    ((dividend as u64 + divisor / 2) / divisor) as u32
}