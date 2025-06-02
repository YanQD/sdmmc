use std::ptr::NonNull;

// 基础错误和数据结构定义
#[derive(Debug)]
pub enum MmcError {
    Timeout,
    InvalidResponse,
    HardwareFault,
    MaxRetries,
}

pub struct MmcCommand {
    pub index: u8,
    pub arg: u32,
    pub flags: u32,
}

pub struct MmcResponse {
    pub resp: [u32; 4],
}

pub struct MmcCard {
    pub cid: [u32; 4],
    pub capacity: u64,
}

// 核心trait，硬件必须实现的基础操作
pub trait HostOpsCore {
    fn send_command_raw(&self, cmd: &MmcCommand) -> Result<MmcResponse, MmcError>;
    fn set_clock_raw(&self, freq: u32) -> Result<(), MmcError>;
    fn reset_controller(&self) -> Result<(), MmcError>;
    fn get_controller_id(&self) -> u32; // 用于区分不同控制器实例
}

// 函数指针类型定义
type CommandFn = fn(&dyn HostOpsCore, &MmcCommand) -> Result<MmcResponse, MmcError>;
type ClockFn = fn(&dyn HostOpsCore, u32) -> Result<(), MmcError>;
type InitFn = fn(&dyn HostOpsCore) -> Result<MmcCard, MmcError>;
type DetectFn = fn(&dyn HostOpsCore) -> bool;

// 标准实现函数
pub fn standard_send_command(host: &dyn HostOpsCore, cmd: &MmcCommand) -> Result<MmcResponse, MmcError> {
    println!("标准命令发送: 带重试机制");
    
    for attempt in 1..=3 {
        println!("  尝试第 {} 次", attempt);
        match host.send_command_raw(cmd) {
            Ok(response) => {
                println!("  命令执行成功");
                return Ok(response);
            }
            Err(MmcError::Timeout) if attempt < 3 => {
                println!("  超时，准备重试");
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            Err(e) => {
                println!("  命令失败: {:?}", e);
                return Err(e);
            }
        }
    }
    
    Err(MmcError::MaxRetries)
}

pub fn standard_set_clock(host: &dyn HostOpsCore, freq: u32) -> Result<(), MmcError> {
    println!("标准时钟设置: {}Hz", freq);
    
    // 标准的时钟设置逻辑：渐进式调整
    let steps = [400_000, freq / 4, freq / 2, freq];
    for step_freq in steps.iter() {
        if *step_freq <= freq {
            host.set_clock_raw(*step_freq)?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    
    println!("  时钟设置完成");
    Ok(())
}

pub fn standard_initialize_card(host: &dyn HostOpsCore) -> Result<MmcCard, MmcError> {
    println!("标准卡片初始化流程");
    
    // 标准初始化序列
    let cmd0 = MmcCommand { index: 0, arg: 0, flags: 0 }; // GO_IDLE_STATE
    host.send_command_raw(&cmd0)?;
    
    let cmd8 = MmcCommand { index: 8, arg: 0x1AA, flags: 0 }; // SEND_IF_COND  
    let _response = host.send_command_raw(&cmd8)?;
    
    println!("  卡片初始化完成");
    Ok(MmcCard {
        cid: [0, 0, 0, 0],
        capacity: 32 * 1024 * 1024 * 1024, // 32GB
    })
}

pub fn standard_card_detect(host: &dyn HostOpsCore) -> bool {
    println!("标准卡片检测");
    // 简单的检测逻辑，实际应该读取寄存器
    true
}

// 自定义实现函数示例
pub fn sdhci_set_clock(host: &dyn HostOpsCore, freq: u32) -> Result<(), MmcError> {
    println!("SDHCI专用时钟设置: {}Hz", freq);
    
    // SDHCI特定的分频计算
    let base_clock = 200_000_000; // 200MHz基础时钟
    let divider = if freq == 0 { 256 } else { (base_clock / freq).max(1) };
    let actual_freq = base_clock / divider;
    
    println!("  计算分频器: {} -> 实际频率: {}Hz", divider, actual_freq);
    host.set_clock_raw(actual_freq)
}

pub fn vendor_initialize_card(host: &dyn HostOpsCore) -> Result<MmcCard, MmcError> {
    println!("厂商定制初始化流程");
    
    // 厂商特定的预处理
    println!("  执行厂商预处理...");
    host.reset_controller()?;
    
    // 调用标准初始化
    let mut card = standard_initialize_card(host)?;
    
    // 厂商特定的后处理
    println!("  执行厂商后处理...");
    card.capacity = card.capacity * 2; // 假设厂商有特殊的容量计算
    
    Ok(card)
}

// 建造者
pub struct HostOpsBuilder {
    command_fn: CommandFn,
    clock_fn: ClockFn,
    init_fn: InitFn,
    detect_fn: DetectFn,
}

impl HostOpsBuilder {
    pub fn new() -> Self {
        Self {
            command_fn: standard_send_command,
            clock_fn: standard_set_clock,
            init_fn: standard_initialize_card,
            detect_fn: standard_card_detect,
        }
    }
    
    pub fn with_command_handler(mut self, f: CommandFn) -> Self {
        self.command_fn = f;
        self
    }
    
    pub fn with_clock_manager(mut self, f: ClockFn) -> Self {
        self.clock_fn = f;
        self
    }
    
    pub fn with_card_initializer(mut self, f: InitFn) -> Self {
        self.init_fn = f;
        self
    }
    
    pub fn with_card_detector(mut self, f: DetectFn) -> Self {
        self.detect_fn = f;
        self
    }
    
    pub fn build(self) -> HostOpsImpl {
        HostOpsImpl {
            command_fn: self.command_fn,
            clock_fn: self.clock_fn,
            init_fn: self.init_fn,
            detect_fn: self.detect_fn,
        }
    }
}

// 实际的HostOps实现
pub struct HostOpsImpl {
    command_fn: CommandFn,
    clock_fn: ClockFn,
    init_fn: InitFn,
    detect_fn: DetectFn,
}

pub trait HostOps {
    fn send_command(&self, host: &dyn HostOpsCore, cmd: &MmcCommand) -> Result<MmcResponse, MmcError>;
    fn set_clock(&self, host: &dyn HostOpsCore, freq: u32) -> Result<(), MmcError>;
    fn initialize_card(&self, host: &dyn HostOpsCore) -> Result<MmcCard, MmcError>;
    fn card_detect(&self, host: &dyn HostOpsCore) -> bool;
}

impl HostOps for HostOpsImpl {
    fn send_command(&self, host: &dyn HostOpsCore, cmd: &MmcCommand) -> Result<MmcResponse, MmcError> {
        (self.command_fn)(host, cmd)
    }
    
    fn set_clock(&self, host: &dyn HostOpsCore, freq: u32) -> Result<(), MmcError> {
        (self.clock_fn)(host, freq)
    }
    
    fn initialize_card(&self, host: &dyn HostOpsCore) -> Result<MmcCard, MmcError> {
        (self.init_fn)(host)
    }
    
    fn card_detect(&self, host: &dyn HostOpsCore) -> bool {
        (self.detect_fn)(host)
    }
}

// MMC Host主结构
pub struct MmcHost {
    controller: Box<dyn HostOpsCore>,
    ops: HostOpsImpl,
    card: Option<MmcCard>,
}

impl MmcHost {
    pub fn new(controller: Box<dyn HostOpsCore>, ops: HostOpsImpl) -> Self {
        Self {
            controller,
            ops,
            card: None,
        }
    }
    
    pub fn send_command(&self, cmd: &MmcCommand) -> Result<MmcResponse, MmcError> {
        self.ops.send_command(&**self.controller, cmd)
    }
    
    pub fn set_clock(&self, freq: u32) -> Result<(), MmcError> {
        self.ops.set_clock(&**self.controller, freq)
    }
    
    pub fn initialize_card(&mut self) -> Result<(), MmcError> {
        let card = self.ops.initialize_card(&**self.controller)?;
        self.card = Some(card);
        Ok(())
    }
    
    pub fn get_card(&self) -> Option<&MmcCard> {
        self.card.as_ref()
    }
}

// 具体控制器实现示例
pub struct SdhciController {
    base_addr: usize,
    controller_id: u32,
}

impl SdhciController {
    pub fn new(base_addr: usize) -> Self {
        Self {
            base_addr,
            controller_id: 0x1234,
        }
    }
}

impl HostOpsCore for SdhciController {
    fn send_command_raw(&self, cmd: &MmcCommand) -> Result<MmcResponse, MmcError> {
        println!("SDHCI发送原始命令: index={}, arg=0x{:08x}", cmd.index, cmd.arg);
        
        // 模拟SDHCI寄存器操作
        if cmd.index == 0 {
            Ok(MmcResponse { resp: [0, 0, 0, 0] })
        } else if cmd.index == 8 {
            Ok(MmcResponse { resp: [0x1AA, 0, 0, 0] })
        } else {
            Ok(MmcResponse { resp: [0xDEADBEEF, 0, 0, 0] })
        }
    }
    
    fn set_clock_raw(&self, freq: u32) -> Result<(), MmcError> {
        println!("SDHCI设置原始时钟: {}Hz (地址: 0x{:x})", freq, self.base_addr);
        Ok(())
    }
    
    fn reset_controller(&self) -> Result<(), MmcError> {
        println!("SDHCI控制器复位");
        Ok(())
    }
    
    fn get_controller_id(&self) -> u32 {
        self.controller_id
    }
}

pub struct DwmmcController {
    mmio_base: usize,
}

impl DwmmcController {
    pub fn new(mmio_base: usize) -> Self {
        Self { mmio_base }
    }
}

impl HostOpsCore for DwmmcController {
    fn send_command_raw(&self, cmd: &MmcCommand) -> Result<MmcResponse, MmcError> {
        println!("DWMMC发送原始命令: index={}", cmd.index);
        // DWMMC特定的实现
        Ok(MmcResponse { resp: [0xCAFEBABE, 0, 0, 0] })
    }
    
    fn set_clock_raw(&self, freq: u32) -> Result<(), MmcError> {
        println!("DWMMC设置原始时钟: {}Hz", freq);
        Ok(())
    }
    
    fn reset_controller(&self) -> Result<(), MmcError> {
        println!("DWMMC控制器复位");
        Ok(())
    }
    
    fn get_controller_id(&self) -> u32 {
        0x5678
    }
}

// 使用示例
fn main() -> Result<(), MmcError> {
    println!("=== MMC驱动框架演示 ===\n");
    
    // 场景1: 标准SDHCI控制器，使用部分标准实现
    println!("场景1: SDHCI控制器 (标准命令 + 自定义时钟)");
    let sdhci_controller = Box::new(SdhciController::new(0x1000_0000));
    let sdhci_ops = HostOpsBuilder::new()
        .with_clock_manager(sdhci_set_clock)  // 使用SDHCI专用时钟设置
        .build();
    
    let mut sdhci_host = MmcHost::new(sdhci_controller, sdhci_ops);
    
    // 设置时钟
    sdhci_host.set_clock(50_000_000)?;
    
    // 发送命令
    let cmd = MmcCommand { index: 0, arg: 0, flags: 0 };
    let _response = sdhci_host.send_command(&cmd)?;
    
    // 初始化卡片
    sdhci_host.initialize_card()?;
    
    if let Some(card) = sdhci_host.get_card() {
        println!("  卡片容量: {} bytes\n", card.capacity);
    }
    
    // 场景2: DWMMC控制器，使用厂商定制初始化
    println!("场景2: DWMMC控制器 (标准实现 + 厂商定制初始化)");
    let dwmmc_controller = Box::new(DwmmcController::new(0x2000_0000));
    let dwmmc_ops = HostOpsBuilder::new()
        .with_card_initializer(vendor_initialize_card)  // 使用厂商定制初始化
        .build();
    
    let mut dwmmc_host = MmcHost::new(dwmmc_controller, dwmmc_ops);
    
    dwmmc_host.initialize_card()?;
    
    if let Some(card) = dwmmc_host.get_card() {
        println!("  厂商定制卡片容量: {} bytes\n", card.capacity);
    }
    
    // 场景3: 完全自定义控制器
    println!("场景3: 完全自定义实现");
    let custom_controller = Box::new(SdhciController::new(0x3000_0000));
    let custom_ops = HostOpsBuilder::new()
        .with_command_handler(|host, cmd| {
            println!("自定义命令处理: index={}", cmd.index);
            host.send_command_raw(cmd)
        })
        .with_clock_manager(|host, freq| {
            println!("自定义时钟管理: {}Hz", freq);
            host.set_clock_raw(freq / 3) // 自定义分频
        })
        .build();
    
    let mut custom_host = MmcHost::new(custom_controller, custom_ops);
    custom_host.set_clock(100_000_000)?;
    
    Ok(())
}