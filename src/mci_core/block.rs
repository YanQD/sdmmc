#[cfg(feature = "dma")]
use dma_api::DVec;

use crate::host::MmcHostResult;

use crate::{
    card::CardType,
    common::commands::{DataBuffer, MmcCommand},
    constants::*,
    host::{MmcHostError, MmcHostOps},
    mci_core::MmcHost,
};

use log::trace;

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

            trace!(
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

        // Extract card information and check if card exists
        let card = match &self.card {
            Some(card) => card,
            None => return Err(MmcHostError::DeviceNotFound),
        };

        // Check if card is properly initialized
        if !card.is_initialized() {
            return Err(MmcHostError::UnsupportedCard);
        }

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

    /// Read one or more data blocks from the card
    #[cfg(feature = "dma")]
    pub fn read_blocks(&self, block_id: u32, blocks: u16, buffer: &mut DVec<u8>) -> MmcHostResult {
        // Check if buffer size matches the expected size based on number of blocks
        let expected_size = blocks as usize * 512;
        if buffer.len() != expected_size {
            return Err(MmcHostError::IoError);
        }

        // Check if card is initialized and extract card information
        let card = match &self.card {
            Some(card) => card,
            None => return Err(MmcHostError::DeviceNotFound),
        };

        // Adjust block address based on card type
        // High capacity cards use block addressing, standard capacity cards use byte addressing
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
            "Reading {} blocks starting at address: {:#x}",
            blocks, card_addr
        );

        // Select appropriate command based on number of blocks
        if blocks == 1 {
            // Single block read operation
            let cmd = MmcCommand::new(MMC_READ_SINGLE_BLOCK, card_addr, MMC_RSP_R1)
                .with_data(512, 1, true); // Configure for reading 512 bytes with DMA
            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Read(buffer)))?;
        } else {
            // Multiple block read operation
            let cmd = MmcCommand::new(MMC_READ_MULTIPLE_BLOCK, card_addr, MMC_RSP_R1)
                .with_data(512, blocks, true); // Configure for reading multiple blocks with DMA

            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Read(buffer)))?;

            // Must send stop transmission command after multiple block read
            let stop_cmd = MmcCommand::new(MMC_STOP_TRANSMISSION, 0, MMC_RSP_R1B);
            self.host_ops().mmc_send_command(&stop_cmd, None)?;
        }

        Ok(())
    }

    /// Write multiple blocks to the card
    #[cfg(feature = "dma")]
    pub fn write_blocks(&self, block_id: u32, blocks: u16, buffer: &DVec<u8>) -> MmcHostResult {
        // Verify that buffer size matches the requested number of blocks
        let expected_size = blocks as usize * 512;
        if buffer.len() != expected_size {
            return Err(MmcHostError::IoError);
        }

        // Extract card information and check if card exists
        let card = match &self.card {
            Some(card) => card,
            None => return Err(MmcHostError::DeviceNotFound),
        };

        // Check if card is properly initialized
        if !card.is_initialized() {
            return Err(MmcHostError::UnsupportedCard);
        }

        // Check if card is write protected
        if self.is_write_protected() {
            return Err(MmcHostError::CommandError);
        }

        // Determine the correct address based on card capacity type
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
                MmcCommand::new(MMC_WRITE_BLOCK, card_addr, MMC_RSP_R1).with_data(512, 1, false); // Configure for writing 512 bytes with DMA (false = write)
            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Write(buffer)))?;
        } else {
            // Multiple block write operation
            let cmd = MmcCommand::new(MMC_WRITE_MULTIPLE_BLOCK, card_addr, MMC_RSP_R1)
                .with_data(512, blocks, false); // Configure for writing multiple blocks

            self.host_ops()
                .mmc_send_command(&cmd, Some(DataBuffer::Write(buffer)))?;

            // Must send stop transmission command after multiple block write
            let stop_cmd = MmcCommand::new(MMC_STOP_TRANSMISSION, 0, MMC_RSP_R1B);
            self.host_ops().mmc_send_command(&stop_cmd, None)?;
        }

        Ok(())
    }
}
