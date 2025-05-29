extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::string::ToString;
use core::fmt::{self, Debug};
use core::ptr::read_volatile;

use bitflags::bitflags;
use log::info;

use crate::delay_us;
use crate::embedded_mmc::host::sdhci::SdhciError;
use crate::embedded_mmc::host::UDevice;

use super::SdhciResult;

#[derive(Debug, Clone)]
pub struct GpioDesc {
    pub pin: u32,
    pub active_low: bool,
    pub enabled: bool,
}

impl Default for GpioDesc {
    fn default() -> Self {
        Self { pin: 0, active_low: false, enabled: false }
    }
}

impl GpioDesc {
    pub fn new(pin: u32, active_low: bool) -> Self {
        Self { pin, active_low, enabled: true }
    }
}

pub trait SdhciOps: Debug + Send + Sync {
    /// 获取卡检测状态
    fn get_cd(&self, host: &SdhciHost) -> SdhciResult<bool>;
    
    /// 设置控制寄存器
    fn set_control_reg(&self, host: &mut SdhciHost);
    
    /// 设置时钟频率
    fn set_clock(&self, host: &mut SdhciHost, clock: u32) -> SdhciResult<()>;
    
    /// 扩展时钟设置
    fn set_clock_ext(&self, host: &mut SdhciHost, div: u32);
    
    /// iOS 后处理配置
    fn set_ios_post(&self, host: &mut SdhciHost);
    
    /// 设置增强选通模式
    fn set_enhanced_strobe(&self, host: &mut SdhciHost) -> SdhciResult<()>;
}

#[derive(Debug)]
pub struct SdhciHost {
    pub name: String,
    pub ioaddr: usize,
    pub quirks: u32,
    pub host_caps: u32,
    pub version: u32,
    pub max_clk: u32,
    pub clk_mul: u32,
    pub clock: u32,
    pub ops: Option<Box<dyn SdhciOps>>,
    pub index: i32,
    pub bus_width: i32,
    pub pwr_gpio: GpioDesc,
    pub cd_gpio: GpioDesc,
    pub voltages: u32,
}

impl SdhciHost {
    pub fn new(name: String) -> Self {
        Self {
            name: name.clone(),
            ioaddr: 0,
            quirks: 0,
            host_caps: 0,
            version: 0,
            max_clk: 0,
            clk_mul: 0,
            clock: 0,
            ops: None,
            index: 0,
            bus_width: 1,
            pwr_gpio: GpioDesc::default(),
            cd_gpio: GpioDesc::default(),
            voltages: 0x00FF8000,
        }
    }

    pub fn set_ops(&mut self, ops: Box<dyn SdhciOps>) {
        self.ops = Some(ops);
    }
}

pub trait SdhciDataOps: Debug + Send + Sync {
    fn emmc_set_clock(&self, host: &mut SdhciHost, clock: u32) -> SdhciResult<()>;
    
    fn set_ios_post(&self, host: &mut SdhciHost);

    fn set_enhanced_strobe(&self, host: &mut SdhciHost) -> SdhciResult<()>;

    fn get_phy(&self, device: &UDevice) -> SdhciResult<()>;
}

#[derive(Debug)]
pub struct StandardSdhciOps;

impl SdhciOps for StandardSdhciOps {
    fn get_cd(&self, host: &SdhciHost) -> SdhciResult<bool> {
        if host.cd_gpio.enabled {
            info!("[Standard] Reading card detect GPIO");
            return Ok(true);
        }
        
        // 从寄存器读取
        info!("[Standard] Reading card detect from register");
        let addr = (host.ioaddr + 0x24) as *const u32;
        let present_state = unsafe { read_volatile(addr) };
        Ok((present_state & 0x00010000) != 0)
    }
    
    fn set_control_reg(&self, host: &mut SdhciHost) {
        info!("[Standard] Setting control register for {}", host.name);
    }
    
    fn set_ios_post(&self, host: &mut SdhciHost) {
        info!("[Standard] Standard iOS post configuration for {}", host.name);
    }
    
    fn set_clock(&self, host: &mut SdhciHost, clock: u32) -> SdhciResult<()> {
        info!("[Standard] Setting standard SDHCI clock to {} Hz", clock);
        host.clock = clock;
        Ok(())
    }
    
    fn set_clock_ext(&self, host: &mut SdhciHost, div: u32) {
        info!("[Standard] Setting extended clock divider: {}", div);
    }
    
    fn set_enhanced_strobe(&self, _host: &mut SdhciHost) -> SdhciResult<()> {
        info!("[Standard] Enhanced strobe not supported");
        Err(SdhciError::UnsupportedOperation)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RkFlags: u32 {
        const DLL_CMD_OUT = 1 << 1;
        const RXCLK_NO_INVERTER = 1 << 2;
        const TAP_VALUE_SEL = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RkTimingConfig {
    pub hs200_tx_tap: u8,
    pub hs400_tx_tap: u8,
    pub hs400_cmd_tap: u8,
    pub hs400_strbin_tap: u8,
    pub ddr50_strbin_delay_num: u8,
}

/// Arasan 数据操作实现 - 对应 C 的 arasan_data
#[derive(Debug)]
pub struct ArasanDataOps {
    pub chip_name: &'static str,
}

impl ArasanDataOps {
    pub fn new() -> Self {
        Self { chip_name: "Arasan-RK3399" }
    }
}

impl SdhciDataOps for ArasanDataOps {
    fn emmc_set_clock(&self, host: &mut SdhciHost, clock: u32) -> SdhciResult<()> {
        info!("[{}] rk3399_sdhci_emmc_set_clock: {} Hz", self.chip_name, clock);
        
        // Arasan 特定的时钟设置逻辑
        if clock > 150_000_000 {
            return Err(SdhciError::ClockSetupFailed);
        }
        
        host.clock = clock;
        Ok(())
    }
    
    fn set_ios_post(&self, host: &mut SdhciHost) {
        info!("[{}] No special iOS post processing", self.chip_name);
    }
    
    fn set_enhanced_strobe(&self, host: &mut SdhciHost) -> SdhciResult<()> {
        info!("[{}] Enhanced strobe not supported", self.chip_name);
        Err(SdhciError::UnsupportedOperation)
    }
    
    fn get_phy(&self, device: &UDevice) -> SdhciResult<()> {
        info!("[{}] rk3399_emmc_get_phy for device: {}", self.chip_name, device.name);
        Ok(())
    }
}

/// DWCMSHC 数据操作实现 - 对应 C 的 rk3568_data 等
#[derive(Debug)]
pub struct DwcmshcDataOps {
    pub chip_name: &'static str,
    pub flags: RkFlags,
    pub timing: RkTimingConfig,
}

impl DwcmshcDataOps {
    pub fn new(chip_name: &'static str, flags: RkFlags, timing: RkTimingConfig) -> Self {
        Self { chip_name, flags, timing }
    }
}

impl SdhciDataOps for DwcmshcDataOps {
    fn emmc_set_clock(&self, host: &mut SdhciHost, clock: u32) -> SdhciResult<()> {
        info!("[{}] dwcmshc_sdhci_emmc_set_clock: {} Hz", self.chip_name, clock);
        
        if clock > 200_000_000 {
            return Err(SdhciError::ClockSetupFailed);
        }
        
        // DWCMSHC 特定的时钟设置逻辑
        if self.flags.contains(RkFlags::RXCLK_NO_INVERTER) {
            info!("  - RXCLK inverter disabled");
        }
        if self.flags.contains(RkFlags::DLL_CMD_OUT) {
            info!("  - DLL CMD OUT enabled");
        }
        if self.flags.contains(RkFlags::TAP_VALUE_SEL) {
            info!("  - TAP value selection enabled");
        }
        
        host.clock = clock;
        Ok(())
    }
    
    fn set_ios_post(&self, host: &mut SdhciHost) {
        info!("[{}] dwcmshc_sdhci_set_ios_post", self.chip_name);
        info!("  - Setting timing parameters:");
        info!("    HS200 TX TAP: {}", self.timing.hs200_tx_tap);
        info!("    HS400 TX TAP: {}", self.timing.hs400_tx_tap);
        info!("    HS400 CMD TAP: {}", self.timing.hs400_cmd_tap);
    }
    
    fn set_enhanced_strobe(&self, host: &mut SdhciHost) -> SdhciResult<()> {
        info!("[{}] dwcmshc_sdhci_set_enhanced_strobe", self.chip_name);
        info!("  - Using strbin tap: {}", self.timing.hs400_strbin_tap);
        Ok(())
    }
    
    fn get_phy(&self, device: &UDevice) -> SdhciResult<()> {
        info!("[{}] dwcmshc_emmc_get_phy for device: {}", self.chip_name, device.name);
        info!("  - DDR50 strbin delay: {}", self.timing.ddr50_strbin_delay_num);
        Ok(())
    }
}

#[derive(Debug)]
pub struct RockchipSdhciOps {
    data_ops: Box<dyn SdhciDataOps>,  // 持有具体芯片实现
    base_ops: Box<dyn SdhciOps>,      // 标准实现作为回退
}

impl RockchipSdhciOps {
    pub fn new(data_ops: Box<dyn SdhciDataOps>, base_ops: Box<dyn SdhciOps>) -> Self {
        Self { data_ops, base_ops }
    }
}

impl SdhciOps for RockchipSdhciOps {
    fn get_cd(&self, host: &SdhciHost) -> SdhciResult<bool> {
        self.base_ops.get_cd(host)
    }
    
    fn set_control_reg(&self, host: &mut SdhciHost) {
        self.base_ops.set_control_reg(host);
    }
    
    fn set_ios_post(&self, host: &mut SdhciHost) {
        info!("[Bridge] rockchip_sdhci_set_ios_post -> calling data_ops");
        self.data_ops.set_ios_post(host);
    }
    
    fn set_clock(&self, host: &mut SdhciHost, clock: u32) -> SdhciResult<()> {
        info!("[Bridge] rockchip_sdhci_set_clock -> calling data_ops.emmc_set_clock");
        self.data_ops.emmc_set_clock(host, clock)
    }
    
    fn set_clock_ext(&self, host: &mut SdhciHost, div: u32) {
        self.base_ops.set_clock_ext(host, div);
    }
    
    fn set_enhanced_strobe(&self, host: &mut SdhciHost) -> SdhciResult<()> {
        info!("[Bridge] rockchip_sdhci_set_enhanced_strobe -> calling data_ops");
        self.data_ops.set_enhanced_strobe(host)
    }
}

#[derive(Clone)]
pub struct SdhciData {
    pub chip_name: &'static str,
    pub data_ops_factory: fn() -> Box<dyn SdhciDataOps>,
}

impl Debug for SdhciData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SdhciData")
            .field("chip_name", &self.chip_name)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct DeviceId {
    pub compatible: String,
    pub data: SdhciData,
}

pub struct SdhciDriver {
    pub name: String,
    pub device_ids: Vec<DeviceId>,
}

impl SdhciDriver {
    pub fn new() -> Self {
        let mut driver = Self {
            name: "rockchip_sdhci_5_1".to_string(),
            device_ids: Vec::new(),
        };
        
        driver.register_devices();
        driver
    }
    
    fn register_devices(&mut self) {
        // Arasan SDHCI 5.1 (RK3399)
        self.device_ids.push(DeviceId {
            compatible: "arasan,sdhci-5.1".to_string(),
            data: SdhciData {
                chip_name: "Arasan-RK3399",
                data_ops_factory: || Box::new(ArasanDataOps::new()),
            },
        });
        
        // Generic DWCMSHC (RK3568)
        self.device_ids.push(DeviceId {
            compatible: "snps,dwcmshc-sdhci".to_string(),
            data: SdhciData {
                chip_name: "DWCMSHC-RK3568",
                data_ops_factory: || Box::new(DwcmshcDataOps::new(
                    "RK3568",
                    RkFlags::RXCLK_NO_INVERTER,
                    RkTimingConfig {
                        hs200_tx_tap: 16,
                        hs400_tx_tap: 8,
                        hs400_cmd_tap: 8,
                        hs400_strbin_tap: 3,
                        ddr50_strbin_delay_num: 16,
                    },
                )),
            },
        });
        
        // RK3588 DWCMSHC
        self.device_ids.push(DeviceId {
            compatible: "rockchip,rk3588-dwcmshc".to_string(),
            data: SdhciData {
                chip_name: "DWCMSHC-RK3588",
                data_ops_factory: || Box::new(DwcmshcDataOps::new(
                    "RK3588",
                    RkFlags::DLL_CMD_OUT,
                    RkTimingConfig {
                        hs200_tx_tap: 16,
                        hs400_tx_tap: 9,  // RK3588 特有
                        hs400_cmd_tap: 8,
                        hs400_strbin_tap: 3,
                        ddr50_strbin_delay_num: 16,
                    },
                )),
            },
        });
        
        // RK3528 DWCMSHC
        self.device_ids.push(DeviceId {
            compatible: "rockchip,rk3528-dwcmshc".to_string(),
            data: SdhciData {
                chip_name: "DWCMSHC-RK3528",
                data_ops_factory: || Box::new(DwcmshcDataOps::new(
                    "RK3528",
                    RkFlags::DLL_CMD_OUT | RkFlags::TAP_VALUE_SEL,
                    RkTimingConfig {
                        hs200_tx_tap: 12,
                        hs400_tx_tap: 6,
                        hs400_cmd_tap: 6,
                        hs400_strbin_tap: 3,
                        ddr50_strbin_delay_num: 10,
                    },
                )),
            },
        });
    }
    
    /// 设备匹配
    pub fn match_device(&self, compatible: &str) -> Option<&SdhciData> {
        for device_id in &self.device_ids {
            if device_id.compatible == compatible {
                info!("✅ Device matched: {} -> {}", 
                        compatible, device_id.data.chip_name);
                return Some(&device_id.data);
            }
        }
        
        info!("❌ No matching device found for: {}", compatible);
        None
    }
    
    /// 设备探测
    pub fn probe(&self, device: &UDevice) -> SdhciResult<SdhciHost> {
        info!("🔍 Probing SDHCI device: {}", device.name);
        
        // 1. 查找匹配的 compatible 字符串
        let device_data = device.compatible.iter()
            .filter_map(|c| self.match_device(c))
            .next()
            .ok_or(SdhciError::DeviceNotFound)?;
        
        // 2. 创建主机实例
        let mut host = SdhciHost::new(device.name.clone());
        
        // 3. 创建数据操作实例
        let data_ops = (device_data.data_ops_factory)();
        
        // 4. 初始化 PHY
        data_ops.get_phy(device)?;
        
        // 5. 创建桥接操作
        let base_ops = Box::new(StandardSdhciOps);
        let bridge_ops = RockchipSdhciOps::new(data_ops, base_ops);
        
        // 6. 设置桥接操作到主机
        host.set_ops(Box::new(bridge_ops));
        
        info!("✅ SDHCI device probed successfully: {} ({})", 
                host.name, device_data.chip_name);
        
        Ok(host)
    }
    
    pub fn list_supported_devices(&self) {
        info!("📋 Supported SDHCI devices:");
        for device_id in &self.device_ids {
            info!("  - {} -> {}", 
                    device_id.compatible, 
                    device_id.data.chip_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_complete_sdhci_flow() -> SdhciResult<()> {
        info!("🚀 Testing complete SDHCI driver flow...\n");
        
        // 1. 创建驱动实例
        let driver = SdhciDriver::new();
        driver.list_supported_devices();
        
        // 2. 创建模拟设备
        let mut rk3568_device = UDevice {
            name: "sdhci@fe2b0000".to_string(),
            compatible: vec!["snps,dwcmshc-sdhci".to_string()],
        };

        let mut host = driver.probe(&mut rk3568_device)?;
        host.set_ioaddr(0xFE2B0000);
        
        // 4. 测试功能调用
        info!("\n🔧 Testing SDHCI operations:");
        
        // 设置时钟 (会调用 dwcmshc_sdhci_emmc_set_clock)
        host.set_clock(150_000_000)?;
        
        // iOS 后处理 (会调用 dwcmshc_sdhci_set_ios_post)
        host.configure_ios_post();
        
        // 增强选通 (会调用 dwcmshc_sdhci_set_enhanced_strobe)
        host.set_enhanced_strobe()?;
        
        // 卡检测
        let card_present = host.is_card_present()?;
        info!("Card present: {}", card_present);
        
        info!("\n✅ Complete SDHCI flow test passed!");
        Ok(())
    }
    
    #[test]
    fn test_different_chips() -> SdhciResult<()> {
        let driver = SdhciDriver::new();
        
        // 测试不同芯片
        let test_devices = vec![
            ("arasan,sdhci-5.1", "RK3399"),
            ("snps,dwcmshc-sdhci", "RK3568"),
            ("rockchip,rk3588-dwcmshc", "RK3588"),
            ("rockchip,rk3528-dwcmshc", "RK3528"),
        ];
        
        for (compatible, chip) in test_devices {
            let mut device = UDevice {
                name: format!("sdhci-{}", chip),
                compatible: vec![compatible.to_string()],
            };
            
            match driver.probe(&mut device) {
                Ok(mut host) => {
                    info!("✅ {} probe successful", chip);
                    
                    // 测试基本操作
                    let _ = host.set_clock(100_000_000);
                    host.configure_ios_post();
                    
                    // 根据芯片测试特定功能
                    match chip {
                        "RK3399" => {
                            // Arasan 不支持增强选通
                            match host.set_enhanced_strobe() {
                                Err(SdhciError::UnsupportedOperation) => {
                                    info!("  ✅ Arasan correctly reports no enhanced strobe");
                                }
                                _ => return Err(SdhciError::HardwareError),
                            }
                        }
                        _ => {
                            // DWCMSHC 系列支持增强选通
                            host.set_enhanced_strobe()?;
                            info!("  ✅ {} enhanced strobe configured", chip);
                        }
                    }
                }
                Err(e) => {
                    info!("❌ {} probe failed: {:?}", chip, e);
                    return Err(e);
                }
            }
        }
        
        info!("\n✅ All chip tests passed!");
        Ok(())
    }
}