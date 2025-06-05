#[macro_export]
macro_rules! impl_register_ops {
    ($struct_name:ident, $field_name:ident) => {
        impl $struct_name {
            #[inline]
            pub fn read_reg8(&self, offset: u32) -> u8 {
                unsafe {
                    core::ptr::read_volatile((self.$field_name + offset as usize) as *const u8)
                }
            }

            #[inline]
            pub fn read_reg16(&self, offset: u32) -> u16 {
                unsafe {
                    core::ptr::read_volatile((self.$field_name + offset as usize) as *const u16)
                }
            }

            #[inline]
            pub fn read_reg32(&self, offset: u32) -> u32 {
                unsafe {
                    core::ptr::read_volatile((self.$field_name + offset as usize) as *const u32)
                }
            }

            #[inline]
            pub fn write_reg8(&self, offset: u32, value: u8) {
                unsafe {
                    core::ptr::write_volatile(
                        (self.$field_name + offset as usize) as *mut u8,
                        value,
                    )
                }
            }

            #[inline]
            pub fn write_reg16(&self, offset: u32, value: u16) {
                unsafe {
                    core::ptr::write_volatile(
                        (self.$field_name + offset as usize) as *mut u16,
                        value,
                    )
                }
            }

            #[inline]
            pub fn write_reg32(&self, offset: u32, value: u32) {
                unsafe {
                    core::ptr::write_volatile(
                        (self.$field_name + offset as usize) as *mut u32,
                        value,
                    )
                }
            }
        }
    };
}
