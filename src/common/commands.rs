use log::info;

#[cfg(feature = "dma")]
use dma_api::DVec;

#[cfg(feature = "pio")]
pub enum DataBuffer<'a> {
    Read(&'a mut [u8]),
    Write(&'a [u8]),
}

#[cfg(feature = "dma")]
pub enum DataBuffer<'a> {
    Read(&'a mut DVec<u8>),
    Write(&'a DVec<u8>),
}

#[derive(Debug)]
pub struct MmcCommand {
    pub opcode: u8,
    pub arg: u32,
    pub resp_type: u32,
    pub data_present: bool,
    pub data_dir_read: bool,
    pub block_size: u16,
    pub block_count: u16,
}

impl MmcCommand {
    pub fn new(opcode: u8, arg: u32, resp_type: u32) -> Self {
        Self {
            opcode,
            arg,
            resp_type,
            data_present: false,
            data_dir_read: true,
            block_size: 0,
            block_count: 0,
        }
    }

    pub fn with_data(mut self, block_size: u16, block_count: u16, is_read: bool) -> Self {
        self.data_present = true;
        self.data_dir_read = is_read;
        self.block_size = block_size;
        self.block_count = block_count;
        self
    }
}

pub struct MmcResponse {
    pub raw: [u32; 4],
}

impl MmcResponse {
    pub fn new() -> Self {
        Self { raw: [0; 4] }
    }

    pub fn as_r1(&self) -> u32 {
        self.raw[0]
    }

    pub fn as_r2(&self) -> [u32; 4] {
        let mut response = [0; 4];
        for i in 0..4 {
            response[i] = self.raw[3 - i] << 8;
            if i != 3 {
                response[i] |= self.raw[3 - i - 1] >> 24;
            }
        }
        info!(
            "eMMC response: {:#x} {:#x} {:#x} {:#x}",
            response[0], response[1], response[2], response[3]
        );

        response
    }

    pub fn as_r3(&self) -> u32 {
        self.raw[0]
    }

    pub fn as_r6(&self) -> u32 {
        self.raw[0]
    }

    pub fn as_r7(&self) -> u32 {
        self.raw[0]
    }
}
