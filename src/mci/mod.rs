//! 注意不应把重名的子模块设为pub
pub mod constants;
pub mod err;
pub mod mci_data;
#[cfg(feature = "dma")]
pub mod mci_dma;
pub mod regs;

mod mci_cmd;
mod mci_cmddata;
mod mci_config;
mod mci_hardware;
mod mci_irq;
mod mci_timing;

#[cfg(feature = "pio")]
mod mci_pio;

pub use err::*;
pub use mci_cmddata::*;
pub use mci_config::*;
pub use mci_timing::*;

use crate::{mci_sleep, regs::*};
use constants::*;
use core::time::Duration;
use log::*;
use regs::*;

#[cfg(feature = "dma")]
use mci_dma::FSdifIDmaDescList;

pub struct MCI {
    config: MCIConfig,
    is_ready: bool,
    prev_cmd: u32, // TODO：这里需要实现成一个实现了Command的enum
    curr_timing: MCITiming,
    #[allow(dead_code)]
    cur_cmd: Option<MCICommand>,
    #[cfg(feature = "dma")]
    desc_list: FSdifIDmaDescList,
}

impl MCI {
    const SWITCH_VOLTAGE: u32 = 11;
    const EXT_APP_CMD: u32 = 55;

    pub(crate) fn relax_handler() {
        mci_sleep(Duration::from_micros(10));
    }

    pub(crate) fn new(config: MCIConfig) -> Self {
        MCI {
            config,
            is_ready: false,
            prev_cmd: 0,
            curr_timing: MCITiming::new(),
            cur_cmd: None,
            #[cfg(feature = "dma")]
            desc_list: FSdifIDmaDescList::new(),
        }
    }

    pub(crate) fn new_restart(config: MCIConfig) -> Self {
        MCI {
            config,
            is_ready: true,
            prev_cmd: 0,
            curr_timing: MCITiming::new(),
            cur_cmd: None,
            #[cfg(feature = "dma")]
            desc_list: FSdifIDmaDescList::new(),
        }
    }
}

/// MCI pub API
impl MCI {
    /// initialization SDIF controller instance
    pub fn config_init(&mut self, config: &MCIConfig) -> MCIResult {
        if self.is_ready {
            warn!("Device is already initialized!!!");
        }
        if *config != self.config {
            self.config = config.clone();
        }
        if self.reset().is_ok() {
            self.is_ready = true;
            info!("Device initialize success !!!");
        }
        Ok(())
    }

    /// deinitialization SDIF controller instance
    pub fn config_deinit(&mut self) -> MCIResult {
        self.interrupt_mask_set(
            MCIInterruptType::GeneralIntr,
            MCIIntMask::ALL_BITS.bits(),
            false,
        ); /* 关闭控制器中断位 */

        self.interrupt_mask_set(
            MCIInterruptType::DmaIntr,
            MCIDMACIntEn::ALL_BITS.bits(),
            false,
        ); /* 关闭DMA中断位 */

        self.raw_status_clear(); /* 清除中断状态 */
        self.dma_status_clear();

        self.power_set(false); /* 关闭电源 */
        self.clock_set(false); /* 关闭卡时钟 */

        let reg = self.config.reg();
        reg.clear_reg(MCIClkSrc::UHS_EXT_CLK_ENA); /* 关闭外部时钟 */
        reg.clear_reg(MCIUhsReg::VOLT_180); /* 恢复为3.3v默认电压 */

        self.is_ready = false;
        Ok(())
    }

    /// Set the Card clock freqency
    pub fn clk_freq_set(&mut self, clk_hz: u32) -> MCIResult {
        let reg = self.config.reg();
        let mut reg_val = MCICmd::UPD_CLK;

        let cmd_reg = reg.read_reg::<MCICmd>();
        let cur_cmd_index = cmd_reg.index_get();

        info!("Set clk as {clk_hz}");
        if cur_cmd_index == Self::SWITCH_VOLTAGE {
            reg_val |= MCICmd::VOLT_SWITCH;
        }

        if clk_hz > 0 {
            /* select board-related time-tuning configurations */
            let target_timing = MCIConfig::get_tuning(clk_hz.into(), self.config.non_removable())
                .ok_or_else(|| {
                error!("No available timing !!!");
                MCIError::InvalidTiming
            })?;

            /* update clock source setting */
            self.update_exteral_clk(MCIClkSrc::from_bits_retain(target_timing.clk_src()))?;

            self.clock_set(false);

            /* update clock for clock source */
            if let Err(err) = if cur_cmd_index == Self::SWITCH_VOLTAGE {
                self.private_cmd11_send(reg_val | cmd_reg)
            } else {
                self.private_cmd_send(reg_val, 0)
            } {
                error!("update ext clock failed !!!");
                return Err(err);
            }

            /* set clock divider */
            reg.write_reg(MCIClkDiv::from_bits_truncate(target_timing.clk_div()));
            reg.write_reg(MCIEnableShift::from_bits_truncate(target_timing.shift()));
            info!(
                "clk_src: 0x{:x} clk_div: 0x{:x}, shift: 0x{:x}",
                reg.read_reg::<MCIClkSrc>(),
                reg.read_reg::<MCIClkDiv>(),
                reg.read_reg::<MCIEnableShift>()
            );

            self.clock_set(true);

            /* update clock for clock divider */
            if cur_cmd_index == Self::SWITCH_VOLTAGE {
                self.private_cmd11_send(reg_val | cmd_reg)?;
            } else {
                self.private_cmd_send(reg_val, 0)?;
            }

            self.curr_timing = target_timing;
        } else {
            /* close bus clock in case target clock is 0 */
            self.clock_set(false);

            if cur_cmd_index == Self::SWITCH_VOLTAGE {
                self.private_cmd11_send(reg_val | cmd_reg)?;
            } else {
                self.private_cmd_send(reg_val, 0)?;
            }

            reg.clear_reg(MCIClkSrc::UHS_EXT_CLK_ENA);

            self.curr_timing = MCITiming::new();
        }
        Ok(())
    }

    /* Read PIO data, it works in IRQ mode */
    // TODO:不知道协议栈层需要不需要调用,已经实现.
    /* Get cmd response and received data after wait poll status or interrupt signal */
    // TODO：不知道协议栈层需要不需要调用,已经实现.

    /* Interrupt handler for SDIF instance */
    // TODO：在中断模式下会使用到
    /* Register event call-back function as handler for interrupt events */
    // TODO：在中断模式下会使用到

    /// Reset controller from error state
    pub fn restart(&self) -> MCIResult {
        if !self.is_ready {
            error!("Device is not yet initialized!!!");
            return Err(MCIError::NotInit);
        }

        /* reset controller */
        self.ctrl_reset(MCICtrl::FIFO_RESET)?;

        /* reset controller if in busy state */
        self.busy_card_reset()?;

        /* reset clock */
        self.clk_restart()?;

        /* reset internal DMA */
        if self.config.trans_mode() == MCITransMode::DMA {
            debug!("DMA enabled, reseting internal DMA!");
            self.idma_reset();
        }
        Ok(())
    }

    /// Dump all register value of SDIF instance
    pub fn register_dump(&self) {
        let reg = self.config.reg();
        warn!("cntrl: 0x{:x}", reg.read_reg::<MCICtrl>());
        warn!("pwren: 0x{:x}", reg.read_reg::<MCIPwrEn>());
        warn!("clkdiv: 0x{:x}", reg.read_reg::<MCIClkDiv>());
        warn!("clkena: 0x{:x}", reg.read_reg::<MCIClkEn>());
        warn!("tmout: 0x{:x}", reg.read_reg::<MCITimeout>());
        warn!("ctype: 0x{:x}", reg.read_reg::<MCICType>());
        warn!("blksz: 0x{:x}", reg.read_reg::<MCIBlkSiz>());
        warn!("blkcnt: 0x{:x}", reg.read_reg::<MCIBytCnt>());
        warn!("intmask: 0x{:x}", reg.read_reg::<MCIIntMask>());
        warn!("cmdarg: 0x{:x}", reg.read_reg::<MCICmdArg>());
        warn!("cmd: 0x{:x}", reg.read_reg::<MCICmd>());
        warn!("resp0: 0x{:x}", reg.read_reg::<MCIResp0>());
        warn!("reps1: 0x{:x}", reg.read_reg::<MCIResp1>());
        warn!("resp2: 0x{:x}", reg.read_reg::<MCIResp2>());
        warn!("resp3: 0x{:x}", reg.read_reg::<MCIResp3>());
        warn!("maskints: 0x{:x}", reg.read_reg::<MCIMaskedInts>());
        warn!("rawints: 0x{:x}", reg.read_reg::<MCIRawInts>());
        warn!("status: 0x{:x}", reg.read_reg::<MCIStatus>());
        warn!("fifoth: 0x{:x}", reg.read_reg::<MCIFifoTh>());
        warn!("carddet: 0x{:x}", reg.read_reg::<MCICardDetect>());
        warn!("wrtprt: 0x{:x}", reg.read_reg::<MCICardWrtp>());
        warn!("cksts: 0x{:x}", reg.read_reg::<MCIClkSts>());
        warn!("trans_cardcnt: 0x{:x}", reg.read_reg::<MCITranCardCnt>());
        warn!("trans_fifocnt: 0x{:x}", reg.read_reg::<MCITranFifoCnt>());
        warn!("debnce: 0x{:x}", reg.read_reg::<MCIDebnce>());
        warn!("uid: 0x{:x}", reg.read_reg::<MCIUid>());
        warn!("vid: 0x{:x}", reg.read_reg::<MCIVid>());
        warn!("hwconf: 0x{:x}", reg.read_reg::<MCIHwconf>());
        warn!("uhsreg: 0x{:x}", reg.read_reg::<MCIUhsReg>());
        warn!("cardreset: 0x{:x}", reg.read_reg::<MCICardReset>());
        warn!("busmode: 0x{:x}", reg.read_reg::<MCIBusMode>());
        warn!("descaddrl: 0x{:x}", reg.read_reg::<MCIDescListAddrL>());
        warn!("descaddrh: 0x{:x}", reg.read_reg::<MCIDescListAddrH>());
        warn!("dmacstatus: 0x{:x}", reg.read_reg::<MCIDMACStatus>());
        warn!("dmacinten: 0x{:x}", reg.read_reg::<MCIDMACIntEn>());
        warn!("curdescaddrl: 0x{:x}", reg.read_reg::<MCICurDescAddrL>());
        warn!("curdescaddrh: 0x{:x}", reg.read_reg::<MCICurDescAddrH>());
        warn!("curbufaddrl: 0x{:x}", reg.read_reg::<MCICurBufAddrL>());
        warn!("curbufaddrh: 0x{:x}", reg.read_reg::<MCIBufAddrH>());
        warn!("card_thrctl: 0x{:x}", reg.read_reg::<MCICardThrctl>());
        warn!("clock_src: 0x{:x}", reg.read_reg::<MCIClkSrc>());
        warn!("emmcddr: 0x{:x}", reg.read_reg::<MCIEmmcDdrReg>());
        warn!("enableshift: 0x{:x}", reg.read_reg::<MCIEnableShift>());
    }

    /// Dump command and data info
    pub fn cmd_info_dump(cmd_data: &MCICommand) {
        debug!("cmd struct @{cmd_data:p}");
        debug!("   opcode: {}", cmd_data.cmdidx());
        debug!("   arg: 0x{:x}", cmd_data.cmdarg());
        let response = cmd_data.get_response();
        debug!(
            "   resp@{:p}: 0x{:x} 0x{:x} 0x{:x} 0x{:x}",
            response, response[0], response[1], response[2], response[3]
        );
        debug!("   flag: 0x{:x}", cmd_data.flag());
        debug!("   data @{:p}", cmd_data.get_data().unwrap());

        if let Some(data) = cmd_data.get_data() {
            debug!("   buf: {:p}, len: {}", data, data.datalen());
            debug!("   blk sz: {}", data.blksz());
            debug!("   blk cnt: {}", data.blkcnt());
        }
    }
}

/// MCI private API
impl MCI {
    fn reset(&self) -> MCIResult {
        /* set fifo */
        self.fifoth_set(
            MCIFifoThDMATransSize::DMATrans8,
            MCIFifoTh::RX_WMARK,
            MCIFifoTh::TX_WMARK,
        );

        /* set card threshold */
        self.cardthr_set(MCIFifoDepth::Depth8);

        /* disable clock and update ext clk */
        self.clock_set(false);

        /* set 1st clock */
        if let Err(err) = self.init_external_clk() {
            error!("Update extern clock failed: {:?}", err);
            return Err(err);
        }

        /* power on */
        self.power_set(true);
        self.clock_set(true);
        self.clock_src_set(true);

        /* set voltage as 3.3v */
        self.voltage_1_8v_set(false);

        /* set bus width as 1 */
        self.bus_width_set(1);

        /* reset controller and card */
        if let Err(err) = if self.config.trans_mode() == MCITransMode::DMA {
            self.ctrl_reset(MCICtrl::FIFO_RESET | MCICtrl::DMA_RESET)
        } else {
            self.ctrl_reset(MCICtrl::FIFO_RESET)
        } {
            error!("Reset controller failed: {:?}", err);
            return Err(err);
        }

        /* send private command to update clock */
        self.private_cmd_send(MCICmd::UPD_CLK, 0)?;

        /* reset card for no-removeable media, e.g. eMMC */
        self.cardreset_config();

        /* clear interrupt status */
        self.clear_interrupt_status();

        /* get base address */
        let reg = self.config.reg();

        /* enable card detect interrupt */
        if !self.config.non_removable() {
            reg.set_reg(MCIIntMask::CD_BIT);
        }

        /* enable controller and internal DMA */
        reg.set_reg(MCICtrl::INT_ENABLE | MCICtrl::USE_INTERNAL_DMAC);

        /* set data and resp timeout */
        reg.write_reg(MCITimeout::timeout_data(
            MCITimeout::MAX_DATA_TIMEOUT,
            MCITimeout::MAX_RESP_TIMEOUT,
        ));

        /* reset descriptors and dma */
        if self.config.trans_mode() == MCITransMode::DMA {
            self.descriptor_set(0);
            self.idma_reset();
        }

        info!("Reset hardware done !!!");
        Ok(())
    }

    fn poll_wait_busy_card(&self) -> MCIResult {
        let busy_bits = MCIStatus::DATA_BUSY | MCIStatus::DATA_STATE_MC_BUSY;
        let reg = self.config.reg();
        let reg_val = reg.read_reg::<MCIStatus>();
        if reg_val.contains(busy_bits) {
            warn!("Card is busy, waiting ...");
        }
        if let Err(err) = reg.retry_for(
            |reg: MCIStatus| {
                let result = !reg.contains(busy_bits);
                MCI::relax_handler();
                result
            },
            Some(RETRIES_TIMEOUT),
        ) {
            error!("Wait card busy timeout !!!");
            return Err(err);
        }
        Ok(())
    }

    fn busy_card_reset(&self) -> MCIResult {
        let reg = self.config.reg();

        reg.set_reg(MCICtrl::CONTROLLER_RESET);

        reg.retry_for(
            |reg_val: MCIStatus| {
                reg.set_reg(MCICtrl::CONTROLLER_RESET);
                !reg_val.contains(MCIStatus::DATA_BUSY)
            },
            Some(RETRIES_TIMEOUT),
        )?;

        Ok(())
    }

    fn clk_restart(&self) -> MCIResult {
        let reg = self.config.reg();

        /* wait command finish if previous command is in error state */
        reg.retry_for(
            |reg| (MCICmd::START & reg).bits() == 0,
            Some(RETRIES_TIMEOUT),
        )?;

        /* update clock */
        self.clock_set(false);

        let clk_div = reg.read_reg::<MCIClkDiv>();
        let uhs = reg.read_reg::<MCIClkSrc>();

        self.update_exteral_clk(uhs)?;

        reg.write_reg(clk_div);

        self.clock_set(true);

        self.private_cmd_send(MCICmd::UPD_CLK, 0)?;
        Ok(())
    }
}
