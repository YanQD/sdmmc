#[cfg(feature = "pio")]
use crate::host::MmcHostResult;
use crate::{
    card::CardType,
    common::commands::{DataBuffer, MmcCommand},
    constants::*,
    core::MmcHost,
    host::{MmcHostError, MmcHostOps},
};
use log::{info, trace};

impl<T: MmcHostOps> MmcHost<T> {
    /// Read blocks from SD card using PIO (Programmed I/O) mode
    /// Parameters:
    /// - block_id: Starting block address to read from
    /// - blocks: Number of blocks to read
    /// - buffer: Buffer to store the read data
    #[cfg(feature = "pio")]
    pub fn read_blocks(&self, block_id: u32, blocks: u16, buffer: &mut [u8]) -> MmcHostResult {
        trace!(
            "pio read_blocks: block_id = {}, blocks = {}",
            block_id, blocks
        );

        // Check if card is initialized
        match &self.card {
            Some(card) => card,
            None => return Err(MmcHostError::DeviceNotFound),
        };

        let card = self.card().unwrap();
        let card_state = if card.card_type() == CardType::Mmc {
            card.cardext().unwrap().as_mmc().unwrap().state
        } else if card.card_type() == CardType::SdV1 || card.card_type() == CardType::SdV2 {
            card.cardext().unwrap().as_sd().unwrap().state
        } else {
            return Err(MmcHostError::InvalidValue);
        };

        // Adjust block address based on card type
        // High capacity cards use block addressing, standard capacity cards use byte addressing
        let card_addr = if card_state & MMC_STATE_HIGHCAPACITY != 0 {
            block_id // High capacity card: use block address directly
        } else {
            block_id * 512 // Standard capacity card: convert to byte address
        };

        trace!(
            "Reading {} blocks starting at address: {:#x}",
            blocks, card_addr
        );

        if blocks == 1 {
            // Single block read operation
            let cmd = MmcCommand::new(MMC_READ_SINGLE_BLOCK, card_addr, MMC_RSP_R1)
                .with_data(512, 1, true);
            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Read(buffer)))
                .unwrap();
        } else {
            // Multiple block read operation
            let cmd = MmcCommand::new(MMC_READ_MULTIPLE_BLOCK, card_addr, MMC_RSP_R1)
                .with_data(512, blocks, true);

            info!(
                "Sending multiple block read command: {:?}, blocks: {}",
                cmd, blocks
            );

            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Read(buffer)))
                .unwrap();

            // Must send stop transmission command after multiple block read
            let stop_cmd = MmcCommand::new(MMC_STOP_TRANSMISSION, 0, MMC_RSP_R1B);
            self.host_ops().mmc_send_command(&stop_cmd, None).unwrap();
        }

        Ok(())
    }

    /// Write blocks to SD card using PIO (Programmed I/O) mode
    /// Parameters:
    /// - block_id: Starting block address to write to
    /// - blocks: Number of blocks to write
    /// - buffer: Buffer containing data to write
    #[cfg(feature = "pio")]
    pub fn write_blocks(&self, block_id: u32, blocks: u16, buffer: &[u8]) -> MmcHostResult {
        trace!(
            "pio write_blocks: block_id = {}, blocks = {}",
            block_id, blocks
        );

        // Check if card is initialized
        match &self.card {
            Some(card) => card,
            None => return Err(MmcHostError::DeviceNotFound),
        };

        // Check if card is write protected
        if self.is_write_protected() {
            return Err(MmcHostError::CommandError);
        }

        let card = self.card().unwrap();
        let card_state = if card.card_type() == CardType::Mmc {
            card.cardext().unwrap().as_mmc().unwrap().state
        } else if card.card_type() == CardType::SdV1 || card.card_type() == CardType::SdV2 {
            card.cardext().unwrap().as_sd().unwrap().state
        } else {
            return Err(MmcHostError::InvalidValue);
        };

        // Determine the correct address based on card capacity type
        let card_addr = if card_state & MMC_STATE_HIGHCAPACITY != 0 {
            block_id // High capacity card: use block address directly
        } else {
            block_id * 512 // Standard capacity card: convert to byte address
        };

        trace!(
            "Writing {} blocks starting at address: {:#x}",
            blocks, card_addr
        );

        // Select appropriate command based on number of blocks
        if blocks == 1 {
            // Single block write operation
            let cmd =
                MmcCommand::new(MMC_WRITE_BLOCK, card_addr, MMC_RSP_R1).with_data(512, 1, false);
            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Write(buffer)))
                .unwrap();
        } else {
            // Multiple block write operation
            let cmd = MmcCommand::new(MMC_WRITE_MULTIPLE_BLOCK, card_addr, MMC_RSP_R1)
                .with_data(512, blocks, false);

            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Write(buffer)))
                .unwrap();

            // Must send stop transmission command after multiple block write
            let stop_cmd = MmcCommand::new(MMC_STOP_TRANSMISSION, 0, MMC_RSP_R1B);
            self.host_ops().mmc_send_command(&stop_cmd, None).unwrap();
        }

        Ok(())
    }
}
