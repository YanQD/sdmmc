use core::time::Duration;

use crate::{
    card::{CardExt, CardType},
    common::commands::{DataBuffer, MmcCommand, MmcResponse},
    constants::*,
    host::{MmcHostError, MmcHostOps, MmcHostResult},
    mci_core::MmcHost,
    mci_sleep,
};
#[cfg(feature = "dma")]
use dma_api::DVec;
use log::{debug, info};

impl<T: MmcHostOps> MmcHost<T> {
    // Send CMD0 to reset the card
    pub fn mmc_go_idle(&self) -> MmcHostResult {
        let cmd = MmcCommand::new(MMC_GO_IDLE_STATE, 0, MMC_RSP_NONE);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();

        mci_sleep(Duration::from_micros(100));

        info!("eMMC reset complete");
        Ok(())
    }

    // Send CMD1 to set OCR and check if card is ready
    pub fn mmc_send_op_cond(
        &mut self,
        ocr: u32,
        mut retry: u32,
        voltages: u32,
    ) -> MmcHostResult<u32> {
        // First command to get capabilities

        let mut cmd = MmcCommand::new(MMC_SEND_OP_COND, ocr, MMC_RSP_R3);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();
        mci_sleep(Duration::from_micros(1000));

        // Get response and store it
        let mut card_ocr = self.get_response().as_r3();

        info!("eMMC first CMD1 response (no args): {:#x}", card_ocr);

        // Calculate arg for next commands
        let ocr_hcs = 0x40000000; // High Capacity Support
        let ocr_busy = 0x80000000;
        let ocr_voltage_mask = 0x007FFF80;
        let ocr_access_mode = 0x60000000;

        let cmd_arg =
            ocr_hcs | (voltages & (card_ocr & ocr_voltage_mask)) | (card_ocr & ocr_access_mode);

        // info!("eMMC CMD1 arg for retries: {:#x}", cmd_arg);

        // Now retry with the proper argument until ready or timeout
        let mut ready = false;
        while retry > 0 && !ready {
            cmd = MmcCommand::new(MMC_SEND_OP_COND, cmd_arg, MMC_RSP_R3);
            self.host_ops().mmc_send_command(&cmd, None).unwrap();
            let resp = self.get_response().as_r3();
            card_ocr = resp;

            info!(
                "CMD1 response raw: {:#x}",
                self.host_ops().read_reg32(EMMC_RESPONSE)
            );
            info!("eMMC CMD1 response: {:#x}", resp);

            // Update card OCR
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

            if !ready {
                retry -= 1;
                // Delay between retries
                mci_sleep(Duration::from_micros(1000));
            }
        }

        info!("eMMC initialization status: {}", ready);

        if !ready {
            return Err(MmcHostError::UnsupportedOperation);
        }

        debug!(
            "Clock control before CMD2: 0x{:x}, stable: {}",
            self.host_ops().read_reg16(EMMC_CLOCK_CONTROL),
            self.is_clock_stable()
        );

        Ok(card_ocr)
    }

    pub fn is_clock_stable(&self) -> bool {
        let clock_ctrl = self.host_ops().read_reg16(EMMC_CLOCK_CONTROL);
        (clock_ctrl & EMMC_CLOCK_INT_STABLE) != 0
    }

    // Send CMD2 to get CID
    pub fn mmc_all_send_cid(&mut self) -> MmcHostResult<[u32; 4]> {
        let cmd = MmcCommand::new(MMC_ALL_SEND_CID, 0, MMC_RSP_R2);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();
        let response = self.get_response();

        // Now borrow card as mutable to update it
        let card = self.card_mut().unwrap();

        card.base_info_mut().set_cid(response.as_r2());

        Ok(card.base_info().cid())
    }

    // Send CMD3 to set RCA for eMMC
    pub fn mmc_set_relative_addr(&self) -> MmcHostResult {
        // Get the RCA value before borrowing the card
        let card = self.card().unwrap();
        let rca = card.base_info().rca();

        let cmd = MmcCommand::new(MMC_SET_RELATIVE_ADDR, rca << 16, MMC_RSP_R1);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();

        Ok(())
    }

    // cmd4 - Set DSR (Driver Stage Register)
    pub fn mmc_set_dsr(&mut self, dsr: u32) -> MmcHostResult {
        // Set DSR (Driver Stage Register) value
        let cmd = MmcCommand::new(MMC_SET_DSR, dsr, MMC_RSP_NONE);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();
        Ok(())
    }

    // Send CMD6 to switch modes
    pub fn mmc_switch(&self, _set: u8, index: u32, value: u8, send_status: bool) -> MmcHostResult {
        let mut retries = 3;
        let cmd = MmcCommand::new(
            MMC_SWITCH,
            (MMC_SWITCH_MODE_WRITE_BYTE << 24) | (index << 16) | ((value as u32) << 8),
            MMC_RSP_R1B,
        );

        loop {
            let ret = self.host_ops().mmc_send_command(&cmd, None);

            if ret.is_ok() {
                debug!("cmd6 {:#x}", self.get_response().as_r1());
                return self.mmc_poll_for_busy(send_status);
            }

            retries -= 1;
            if retries <= 0 {
                debug!("Switch command failed after 3 retries");
                break;
            }
        }

        Err(MmcHostError::Timeout)
    }

    #[cfg(feature = "dma")]
    pub fn mmc_send_ext_csd(&mut self, ext_csd: &mut DVec<u8>) -> MmcHostResult {
        let cmd = MmcCommand::new(MMC_SEND_EXT_CSD, 0, MMC_RSP_R1).with_data(
            MMC_MAX_BLOCK_LEN as u16,
            1,
            true,
        );

        self.host_ops()
            .mmc_send_command(&cmd, Some(DataBuffer::Read(ext_csd)))?;

        // debug!("CMD8: {:#x}",self.get_response().as_r1());
        // debug!("EXT_CSD read successfully, rev: {}", ext_csd[EXT_CSD_REV as usize]);

        Ok(())
    }

    // Send CMD8 to get EXT_CSD
    #[cfg(feature = "pio")]
    pub fn mmc_send_ext_csd(&mut self, ext_csd: &mut [u8; 512]) -> MmcHostResult {
        let cmd = MmcCommand::new(MMC_SEND_EXT_CSD, 0, MMC_RSP_R1).with_data(
            MMC_MAX_BLOCK_LEN as u16,
            1,
            true,
        );

        self.host_ops()
            .mmc_send_command(&cmd, Some(DataBuffer::Read(ext_csd)))
            .unwrap();

        // debug!("CMD8: {:#x}",self.get_response().as_r1());
        // debug!("EXT_CSD read successfully, rev: {}", ext_csd[EXT_CSD_REV as usize]);

        Ok(())
    }

    // Send CMD9 to get CSD
    pub fn mmc_send_csd(&mut self) -> MmcHostResult<[u32; 4]> {
        // Get the RCA value before borrowing the card
        let card = self.card().unwrap();
        let rca = card.base_info().rca();

        let cmd = MmcCommand::new(MMC_SEND_CSD, rca << 16, MMC_RSP_R2);
        self.host_ops().mmc_send_command(&cmd, None).unwrap();
        let response = self.get_response();

        // Now borrow card as mutable to update it
        let card = self.card_mut().unwrap();
        card.base_info_mut().set_csd(response.as_r2());

        Ok(card.base_info().csd())
    }

    // Get response from the last command
    pub fn get_response(&self) -> MmcResponse {
        let mut response = MmcResponse::new();
        response.raw[0] = self.host_ops().read_reg32(EMMC_RESPONSE);
        response.raw[1] = self.host_ops().read_reg32(EMMC_RESPONSE + 4);
        response.raw[2] = self.host_ops().read_reg32(EMMC_RESPONSE + 8);
        response.raw[3] = self.host_ops().read_reg32(EMMC_RESPONSE + 12);

        response
    }
}
