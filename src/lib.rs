//! # ASTC Encoding
//!
//! This is a library to encode images as ASTC for use on a GPU with hardware compression support.
//! It is implemented as bindings to ARM's official `astc-encoder` library.
//!
//! In order to use the images generated by this library directly on the GPU, you need ensure that
//! the GPU you're running on has support for ASTC, which can be queried with the Vulkan
//! `textureCompressionASTC_*` flags (one flag for each of the modes in `Profile`).

#![warn(missing_docs)]

use std::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    os::raw::c_void,
    ptr::NonNull,
};

/// An error during initialization, compression or decompression.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Error {
    /// The block size is out of range of the supported sizes.
    BadBlockSize,
    /// > TODO: The context is broken somehow
    BadContext,
    /// > TODO: The CPU has incomplete float support somehow
    BadCpuFloat,
    /// The library was compiled for ISA incompatible with the ISA that we're running on.
    BadDecodeMode,
    /// The flags are contradictory or otherwise incorrect.
    BadFlags,
    /// A bad parameter was supplied
    BadParam,
    /// The supplied preset is unsupported
    BadQuality,
    /// The supplied profile is unsupported
    BadProfile,
    /// The supplied swizzle is unsupported
    BadSwizzle,
    /// Some unimplemented code was reached
    NotImplemented,
    /// We ran out of memory
    OutOfMem,
    /// Something else went wrong (this should never happen!)
    Unknown,
}

fn error_code_to_result(code: astcenc_sys::astcenc_error) -> Result<(), Error> {
    match code {
        astcenc_sys::astcenc_error_ASTCENC_SUCCESS => Ok(()),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_BLOCK_SIZE => Err(Error::BadBlockSize),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_CONTEXT => Err(Error::BadContext),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_CPU_FLOAT => Err(Error::BadCpuFloat),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_DECODE_MODE => Err(Error::BadDecodeMode),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_FLAGS => Err(Error::BadFlags),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_PARAM => Err(Error::BadParam),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_QUALITY => Err(Error::BadQuality),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_PROFILE => Err(Error::BadProfile),
        astcenc_sys::astcenc_error_ASTCENC_ERR_BAD_SWIZZLE => Err(Error::BadSwizzle),
        astcenc_sys::astcenc_error_ASTCENC_ERR_NOT_IMPLEMENTED => Err(Error::NotImplemented),
        astcenc_sys::astcenc_error_ASTCENC_ERR_OUT_OF_MEM => Err(Error::OutOfMem),
        _ => Err(Error::Unknown),
    }
}

/// The core context. All configuration should be done through this.
pub struct Context {
    inner: NonNull<astcenc_sys::astcenc_context>,
    config: Config,
}

unsafe impl Sync for Context {}
unsafe impl Send for Context {}

impl Default for Context {
    fn default() -> Self {
        Self::new(Config::default()).unwrap()
    }
}

/// A 3-dimensional set of width, height and depth. ASTC supports 3D images, so we
/// always have to specify the depth of an image.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Extents {
    /// Width
    pub x: u32,
    /// Height
    pub y: u32,
    /// Depth
    pub z: u32,
}

impl Extents {
    /// The block size of the image, by default. This default block size assumes a 2D image,
    /// and so sets the depth to 1, making the default block size 4x4x1.
    pub fn default_block_size() -> Self {
        Self::new(4, 4)
    }

    /// Create a 2D extent (depth set to 1)
    pub fn new(x: u32, y: u32) -> Self {
        Self { x, y, z: 1 }
    }

    /// Create a 3D extent
    pub fn new_3d(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }
}

/// The performance preset, higher settings take more time but provide higher quality.
/// It will _not_ provide better compression at higher settings, compression is decided
/// only by the block size.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct Preset(f32);

impl Default for Preset {
    fn default() -> Self {
        Self(astcenc_sys::ASTCENC_PRE_MEDIUM)
    }
}

/// The fastest, lowest quality, search preset.
pub const PRESET_FASTEST: Preset = Preset(astcenc_sys::ASTCENC_PRE_FASTEST);
/// The fast search preset.
pub const PRESET_FAST: Preset = Preset(astcenc_sys::ASTCENC_PRE_FAST);
/// The medium quality search preset.
pub const PRESET_MEDIUM: Preset = Preset(astcenc_sys::ASTCENC_PRE_MEDIUM);
/// The thorough quality search preset.
pub const PRESET_THOROUGH: Preset = Preset(astcenc_sys::ASTCENC_PRE_THOROUGH);
/// The thorough quality search preset.
pub const PRESET_VERY_THOROUGH: Preset = Preset(astcenc_sys::ASTCENC_PRE_VERYTHOROUGH);
/// The exhaustive, highest quality, search preset.
pub const PRESET_EXHAUSTIVE: Preset = Preset(astcenc_sys::ASTCENC_PRE_EXHAUSTIVE);

/// The color profile. HDR and LDR SRGB require the image to use floats for its individual colors.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Profile {
    /// HDR in all 4 components.
    HdrRgba,
    /// HDR, but with LDR clamped 0..1.
    HdrRgbLdrA,
    /// LDR in all 4 components.
    LdrRgba,
    /// Signed LDR.
    LdrSrgb,
}

impl Default for Profile {
    fn default() -> Self {
        Self::LdrRgba
    }
}

impl Profile {
    fn into_sys(self) -> astcenc_sys::astcenc_profile {
        match self {
            Self::HdrRgba => astcenc_sys::astcenc_profile_ASTCENC_PRF_HDR,
            Self::HdrRgbLdrA => astcenc_sys::astcenc_profile_ASTCENC_PRF_HDR_RGB_LDR_A,
            Self::LdrRgba => astcenc_sys::astcenc_profile_ASTCENC_PRF_LDR,
            Self::LdrSrgb => astcenc_sys::astcenc_profile_ASTCENC_PRF_LDR_SRGB,
        }
    }
}

/// Configuration for initializing `Context`, see `ConfigBuilder` for more information.
pub struct Config {
    inner: astcenc_sys::astcenc_config,
}

impl Default for Config {
    fn default() -> Self {
        ConfigBuilder::default().build().unwrap()
    }
}

/// Builder for the context configuration.
#[derive(Clone)]
pub struct ConfigBuilder {
    profile: Profile,
    preset: Preset,
    block_size: Extents,
    flags: Flags,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            profile: Profile::default(),
            preset: Preset::default(),
            block_size: Extents::default_block_size(),
            flags: Flags::default(),
        }
    }
}

impl ConfigBuilder {
    /// Create a new, default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the color profile, i.e. the accepted range of the input components.
    pub fn profile(&mut self, profile: Profile) -> &mut Self {
        self.profile = profile;
        self
    }

    /// Set the color profile, i.e. the accepted range of the input components.
    pub fn with_profile(mut self, profile: Profile) -> Self {
        self.profile(profile);
        self
    }

    /// Set the flags to use when initializing `astcenc`.
    pub fn flags(&mut self, flags: Flags) -> &mut Self {
        self.flags = flags;
        self
    }

    /// Set the flags to use when initializing `astcenc`.
    pub fn with_flags(mut self, flags: Flags) -> Self {
        self.flags(flags);
        self
    }

    /// Set the preset, i.e. the balance between speed and quality (*not* speed and
    /// compression ratio, compression ratio, compression ratio is decided by the block
    /// size).
    pub fn preset(&mut self, preset: Preset) -> &mut Self {
        self.preset = preset;
        self
    }

    /// Set the preset, i.e. the balance between speed and quality (*not* speed and
    /// compression ratio, compression ratio, compression ratio is decided by the block
    /// size).
    pub fn with_preset(mut self, preset: Preset) -> Self {
        self.preset(preset);
        self
    }

    /// Set the block size, which decides the compression ratio for the image. Each block
    /// uses 16 bytes of memory.
    pub fn block_size(&mut self, block_size: Extents) -> &mut Self {
        self.block_size = block_size;
        self
    }

    /// Set the block size, which decides the compression ratio for the image. Each block
    /// uses 16 bytes of memory.
    pub fn with_block_size(mut self, block_size: Extents) -> Self {
        self.block_size(block_size);
        self
    }

    /// Create the config from these settings.
    pub fn build(self) -> Result<Config, Error> {
        let mut cfg: MaybeUninit<astcenc_sys::astcenc_config> = MaybeUninit::uninit();

        error_code_to_result(unsafe {
            astcenc_sys::astcenc_config_init(
                self.profile.into_sys(),
                self.block_size.x,
                self.block_size.y,
                self.block_size.z,
                self.preset.0,
                self.flags.into_sys(),
                cfg.as_mut_ptr(),
            )
        })?;

        Ok(Config {
            inner: unsafe { cfg.assume_init() },
        })
    }
}

/// Which of the supported subpixel types the image data's subpixels should be interpreted as.
/// Floating-point types must be used for HDR data.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    /// Half-size floats (see `half::f16`)
    F16,
    /// Normal floats
    F32,
    /// Individual bytes.
    U8,
}

impl Type {
    fn into_sys(self) -> astcenc_sys::astcenc_type {
        match self {
            Self::F16 => astcenc_sys::astcenc_type_ASTCENC_TYPE_F16,
            Self::F32 => astcenc_sys::astcenc_type_ASTCENC_TYPE_F32,
            Self::U8 => astcenc_sys::astcenc_type_ASTCENC_TYPE_U8,
        }
    }
}

/// A valid type for a subpixel.
pub trait DataType: Sized {
    /// The runtime subpixel type associated with this compile-time type.
    const TYPE: Type;

    /// Convert an immutable array of `Self` to bytes.
    fn as_u8s(array: &[Self]) -> &[u8];
    /// Convert a mutable array of `Self` to bytes.
    fn as_u8s_mut(array: &mut [Self]) -> &mut [u8];
}

impl DataType for u8 {
    const TYPE: Type = Type::U8;

    fn as_u8s(array: &[Self]) -> &[u8] {
        array
    }

    fn as_u8s_mut(array: &mut [Self]) -> &mut [u8] {
        array
    }
}

impl DataType for f32 {
    const TYPE: Type = Type::F32;

    fn as_u8s(array: &[Self]) -> &[u8] {
        unsafe { std::mem::transmute(array) }
    }

    fn as_u8s_mut(array: &mut [Self]) -> &mut [u8] {
        unsafe { std::mem::transmute(array) }
    }
}

impl DataType for half::f16 {
    const TYPE: Type = Type::F16;

    fn as_u8s(array: &[Self]) -> &[u8] {
        unsafe { std::mem::transmute(array) }
    }

    fn as_u8s_mut(array: &mut [Self]) -> &mut [u8] {
        unsafe { std::mem::transmute(array) }
    }
}

/// The 3D image type. Each pixel should be RGBA. The data can be anything that dereferences to a
/// flat array of color components, as long as the color components are in one of the supported
/// formats. For HDR images, `f32` or `half::f16` must be used.
#[derive(Default)]
pub struct Image<T> {
    /// The dimensions of the image. This _must_ match the length of the data.
    pub extents: Extents,
    /// The data array.
    pub data: T,
}

/// An individual component of a swizzle.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Selector {
    /// Select the red component
    Red,
    /// Select the green component
    Green,
    /// Select the blue component
    Blue,
    /// Select the alpha component
    Alpha,
    /// Select the z component, which is calculated using trigonometry based on the red
    /// and green components.
    Z,
    /// Constant 1.
    One,
    /// Constant 0.
    Zero,
}

impl Selector {
    fn into_sys(self) -> astcenc_sys::astcenc_swz {
        match self {
            Self::Red => astcenc_sys::astcenc_swz_ASTCENC_SWZ_R,
            Self::Green => astcenc_sys::astcenc_swz_ASTCENC_SWZ_G,
            Self::Blue => astcenc_sys::astcenc_swz_ASTCENC_SWZ_B,
            Self::Alpha => astcenc_sys::astcenc_swz_ASTCENC_SWZ_A,
            Self::Z => astcenc_sys::astcenc_swz_ASTCENC_SWZ_Z,
            Self::One => astcenc_sys::astcenc_swz_ASTCENC_SWZ_1,
            Self::Zero => astcenc_sys::astcenc_swz_ASTCENC_SWZ_0,
        }
    }
}

/// A component selection swizzle. The image must always be in RGBA order, even if the G, B
/// and/or A components are never used.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Swizzle {
    /// The component to use for the red channel.
    pub r: Selector,
    /// The component to use for the green channel.
    pub g: Selector,
    /// The component to use for the blue channel.
    pub b: Selector,
    /// The component to use for the alpha channel.
    pub a: Selector,
}

impl Swizzle {
    /// Default swizzle for greyscale without alpha.
    ///
    /// To access the output in a shader, use the `.g` swizzle.
    pub fn rrr1() -> Self {
        Self {
            r: Selector::Red,
            g: Selector::Red,
            b: Selector::Red,
            a: Selector::One,
        }
    }

    /// Default swizzle for greyscale with alpha.
    ///
    /// To access the output in a shader, use the `.ga` swizzle.
    pub fn rrrg() -> Self {
        Self {
            r: Selector::Red,
            g: Selector::Red,
            b: Selector::Red,
            a: Selector::Green,
        }
    }

    /// Default swizzle for RGB without alpha.
    ///
    /// To access the output in a shader, use the `.rga` swizzle.
    pub fn rgb1() -> Self {
        Self {
            r: Selector::Red,
            g: Selector::Green,
            b: Selector::Blue,
            a: Selector::One,
        }
    }

    /// Default swizzle for RGB with alpha.
    ///
    /// To access the output in a shader, use the `.rga` swizzle.
    pub fn rgba() -> Self {
        Self {
            r: Selector::Red,
            g: Selector::Green,
            b: Selector::Blue,
            a: Selector::Alpha,
        }
    }

    fn into_sys(self) -> astcenc_sys::astcenc_swizzle {
        astcenc_sys::astcenc_swizzle {
            r: self.r.into_sys(),
            g: self.g.into_sys(),
            b: self.b.into_sys(),
            a: self.a.into_sys(),
        }
    }
}

impl Context {
    /// Create a new context from the given config (see `ConfigBuilder` for more information on this
    /// config), and sets the number of threads that the internal encoder/decoder will use. See
    /// [`Context::new`], which does the same thing but automatically guesses the correct thread count
    /// by trying to use 8 threads but falling back to the number of cores if it is less than
    /// 8. Returns an error in the case that the config is invalid or the context could not be
    /// allocated.
    pub fn with_threads(threads: usize, config: Config) -> Result<Self, Error> {
        let mut cfg: MaybeUninit<*mut astcenc_sys::astcenc_context> = MaybeUninit::uninit();

        error_code_to_result(unsafe {
            astcenc_sys::astcenc_context_alloc(&config.inner, threads as _, cfg.as_mut_ptr())
        })?;

        Ok(Self {
            inner: unsafe { NonNull::new(cfg.assume_init()).ok_or(Error::Unknown)? },
            config,
        })
    }

    /// Create a new context from the given config (see `ConfigBuilder` for more information on this
    /// config). Returns an error in the case that the config is invalid or the context could not be
    /// allocated.
    pub fn new(config: Config) -> Result<Self, Error> {
        const MAX_THREADS: usize = 8;

        let threads = num_cpus::get().min(MAX_THREADS);

        Self::with_threads(threads, config)
    }

    /// Compress the given image, returning a byte vector that can be sent to the GPU.
    pub fn compress<D, T, L>(
        &mut self,
        image: &Image<T>,
        swizzle: Swizzle,
    ) -> Result<Vec<u8>, Error>
    where
        D: DataType,
        T: Deref<Target = [L]>,
        L: Deref<Target = [D]>,
    {
        const BYTES_PER_BLOCK: usize = 16;

        if image.data.len() != image.extents.z as usize {
            return Err(Error::BadParam);
        }

        if image
            .data
            .iter()
            .any(|layer| layer.len() != (image.extents.x * image.extents.y * 4) as usize)
        {
            return Err(Error::BadParam);
        }

        let blocks_x = image.extents.x.div_ceil(self.config.inner.block_x);
        let blocks_y = image.extents.y.div_ceil(self.config.inner.block_y);
        let blocks_z = image.extents.z.div_ceil(self.config.inner.block_z);

        let bytes = blocks_x as usize * blocks_y as usize * blocks_z as usize * BYTES_PER_BLOCK;
        let mut out = Vec::with_capacity(bytes);

        let mut image_data_pointers = image
            .data
            .iter()
            .map(|layer| layer.as_ptr() as *const c_void)
            .collect::<Vec<_>>();
        let mut image_sys = astcenc_sys::astcenc_image {
            dim_x: image.extents.x,
            dim_y: image.extents.y,
            dim_z: image.extents.z,
            data_type: D::TYPE.into_sys(),
            data: image_data_pointers.as_mut_ptr() as *mut *mut _,
        };

        error_code_to_result(unsafe {
            astcenc_sys::astcenc_compress_image(
                self.inner.as_mut(),
                &mut image_sys as *mut _,
                &swizzle.into_sys(),
                out.as_mut_ptr(),
                bytes,
                0,
            )
        })?;

        unsafe { out.set_len(bytes) };

        self.compress_reset()?;

        Ok(out)
    }

    /// Decompress an image into a pre-existing buffer. The metadata (size and border padding) must
    /// already be set and enough space must be reserved in `out.data` for the output pixels (RGBA).
    pub fn decompress_into<D, T, L>(
        &mut self,
        data: &[u8],
        out: &mut Image<T>,
        swizzle: Swizzle,
    ) -> Result<(), Error>
    where
        D: DataType,
        T: DerefMut<Target = [L]>,
        L: DerefMut<Target = [D]>,
    {
        let mut image_data_pointers = out
            .data
            .iter_mut()
            .map(|layer| layer.as_mut_ptr() as *mut c_void)
            .collect::<Vec<_>>();
        let mut image_sys = astcenc_sys::astcenc_image {
            dim_x: out.extents.x,
            dim_y: out.extents.y,
            dim_z: out.extents.z,
            data_type: D::TYPE.into_sys(),
            data: image_data_pointers.as_mut_ptr(),
        };

        error_code_to_result(unsafe {
            astcenc_sys::astcenc_decompress_image(
                self.inner.as_mut(),
                data.as_ptr(),
                data.len(),
                &mut image_sys,
                &swizzle.into_sys(),
                0,
            )
        })?;

        self.decompress_reset()?;

        Ok(())
    }

    /// Decompress an image. The metadata is not stored in the compressed data itself, and should be
    /// stored as a separate header.
    pub fn decompress<D>(
        &mut self,
        data: &[u8],
        extents: Extents,
        swizzle: Swizzle,
    ) -> Result<Image<Vec<Vec<D>>>, Error>
    where
        D: DataType,
    {
        let size_2d = (extents.x * extents.y * 4) as usize;
        let mut out = Image {
            extents,
            data: (0..extents.z)
                .map(|_| Vec::with_capacity(size_2d))
                .collect::<Vec<Vec<D>>>(),
        };

        let mut image_data_pointers = out
            .data
            .iter_mut()
            .map(|layer| layer.as_mut_ptr() as *mut c_void)
            .collect::<Vec<_>>();
        let mut image_sys = astcenc_sys::astcenc_image {
            dim_x: out.extents.x,
            dim_y: out.extents.y,
            dim_z: out.extents.z,
            data_type: D::TYPE.into_sys(),
            data: image_data_pointers.as_mut_ptr(),
        };

        error_code_to_result(unsafe {
            astcenc_sys::astcenc_decompress_image(
                self.inner.as_mut(),
                data.as_ptr(),
                data.len(),
                &mut image_sys,
                &swizzle.into_sys(),
                0,
            )
        })?;

        for layer in &mut out.data {
            unsafe { layer.set_len(size_2d) };
        }

        self.decompress_reset()?;

        Ok(out)
    }

    fn compress_reset(&mut self) -> Result<(), Error> {
        error_code_to_result(unsafe { astcenc_sys::astcenc_compress_reset(self.inner.as_mut()) })
    }

    fn decompress_reset(&mut self) -> Result<(), Error> {
        error_code_to_result(unsafe { astcenc_sys::astcenc_decompress_reset(self.inner.as_mut()) })
    }
}

bitflags::bitflags! {
    #[derive(Copy, Clone, PartialEq, Eq)]
    /// Configuration flags for the context.
    pub struct Flags: std::os::raw::c_uint {
        /// Treat the image as a 2-component normal map for the purposes of error calculation.
        /// Z will always be recalculated.
        const MAP_NORMAL       = astcenc_sys::ASTCENC_FLG_MAP_NORMAL;
        /// The decode_unorm8 decode mode rounds differently to the decode_fp16 decode mode, so enabling this
        /// flag during compression will allow the compressor to use the correct rounding when selecting
        /// encodings. This will improve the compressed image quality if your application is using the
        /// decode_unorm8 decode mode, but will reduce image quality if using decode_fp16.
        const DECODE_UNORM8 = astcenc_sys::ASTCENC_FLG_USE_DECODE_UNORM8;
        /// Weight any error in the RGB components by the A component, which leads to better
        /// quality in areas with higher alpha by comparison.
        const USE_ALPHA_WEIGHT = astcenc_sys::ASTCENC_FLG_USE_ALPHA_WEIGHT;
        /// Calculate error using a perceptual algorithm instead of peak signal-to-noise ratio,
        /// best used for normal maps. Not all input types support perceptual error calculation
        /// at all.
        const USE_PERCEPTUAL   = astcenc_sys::ASTCENC_FLG_USE_PERCEPTUAL;
        /// Disable compression support. Uses less memory.
        const DECOMPRESS_ONLY  = astcenc_sys::ASTCENC_FLG_DECOMPRESS_ONLY;
        /// Only guarantee decompression of images which have been compressed using the current context.
        /// This enables additional optimizations.
        const SELF_DECOMPRESS_ONLY = astcenc_sys::ASTCENC_FLG_SELF_DECOMPRESS_ONLY;
        /// Input data will be treated as HDR data that has been stored in an LDR RGBM-encoded wrapper
        /// format. Data must be preprocessed by the user to be in LDR RGBM format before calling the
        /// compression function, this flag is only used to control the use of RGBM-specific heuristics and
        /// error metrics.
        const MAP_RGBM = astcenc_sys::ASTCENC_FLG_MAP_RGBM;
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { astcenc_sys::astcenc_context_free(self.inner.as_ptr()) };
    }
}

impl Flags {
    fn into_sys(self) -> std::os::raw::c_uint {
        self.bits()
    }
}

impl Default for Flags {
    fn default() -> Self {
        Flags::USE_ALPHA_WEIGHT
    }
}

#[cfg(test)]
mod tests {
    use crate::{ConfigBuilder, Context, DataType, Extents, Flags, Image, Preset, Swizzle};
    use all_asserts::assert_lt;
    use average::Mean;
    use half::f16;
    use rand::{rngs::StdRng, Rng, SeedableRng};

    trait TestType: DataType + Sized {
        fn abs_diff(this: &[Self], other: &[Self]) -> f32;
        fn make_rand<T: Rng>(rand: &mut T) -> Self;
        fn flags() -> Flags {
            Flags::default() | Flags::SELF_DECOMPRESS_ONLY
        }
    }

    impl TestType for u8 {
        fn abs_diff(this: &[Self], other: &[Self]) -> f32 {
            this.iter()
                .zip(other)
                .map(|(&a, &b)| ((a as f32 - b as f32) / u8::MAX as f32).powi(2))
                .sum::<f32>()
                .sqrt()
        }

        fn make_rand<T: Rng>(rand: &mut T) -> Self {
            rand.gen()
        }

        fn flags() -> Flags {
            Flags::default() | Flags::SELF_DECOMPRESS_ONLY | Flags::DECODE_UNORM8
        }
    }

    impl TestType for f32 {
        fn abs_diff(this: &[Self], other: &[Self]) -> f32 {
            this.iter()
                .zip(other)
                .map(|(&a, &b)| (a - b).powi(2))
                .sum::<f32>()
                .sqrt()
        }
        fn make_rand<T: Rng>(rand: &mut T) -> Self {
            rand.gen()
        }
    }

    impl TestType for f16 {
        fn abs_diff(this: &[Self], other: &[Self]) -> f32 {
            this.iter()
                .zip(other)
                .map(|(&a, &b)| (a.to_f32() - b.to_f32()).powi(2))
                .sum::<f32>()
                .sqrt()
        }
        fn make_rand<T: Rng>(rand: &mut T) -> Self {
            f16::from_f32(rand.gen::<f32>())
        }
    }

    fn test_preset_image(preset: Preset, tolerance: f64) {
        const IMAGE_BYTES: &[u8] =
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/image.raw"));
        const IMAGE_EXTENTS: Extents = Extents {
            x: 640,
            y: 975,
            z: 1,
        };

        let img = Image {
            extents: IMAGE_EXTENTS,
            data: &[IMAGE_BYTES][..],
        };

        let mut ctx = Context::new(
            ConfigBuilder::new()
                .with_flags(u8::flags())
                .with_preset(preset)
                .build()
                .unwrap(),
        )
        .unwrap();
        let swz = Swizzle::rgba();

        let data = ctx.compress(&img, swz).unwrap();

        let img2 = ctx.decompress(&data, img.extents, swz).unwrap();

        assert_eq!(img.extents, img2.extents);
        assert_eq!(img.data.len(), img2.data.len());
        for datas in img.data.iter().zip(img2.data.iter()) {
            // Assert that we have a flat u8 array, just to make sure this stops compiling
            // if the inner data is converted to a 2D array.
            let (original_data, decompressed_data): (&&[u8], &Vec<u8>) = datas;
            assert_eq!(original_data.len(), decompressed_data.len());
        }

        // ASTC being a lossy compression algorithm, we can't compare the data between the two
        // images, but we make sure that the two images are somewhat close.
        assert_lt!(
            img.data
                .iter()
                .flat_map(|frame| frame.chunks(4))
                .zip(img2.data.iter().flat_map(|frame| frame.chunks(4)))
                .map(|(px1, px2)| TestType::abs_diff(px1, px2) as f64)
                .collect::<Mean>()
                .mean(),
            tolerance
        );
    }

    fn test_preset_noise<T>(preset: Preset, extents: Extents, tolerance: f64)
    where
        T: TestType,
    {
        let mut img = Image::<Vec<Vec<T>>>::default();
        let mut rand = StdRng::from_seed(Default::default());
        img.extents = extents;
        let Extents {
            x: width,
            y: height,
            z: depth,
        } = extents;
        img.data.resize_with(depth as usize, || {
            (0..width * height * 4)
                .map(|_| T::make_rand(&mut rand))
                .collect::<Vec<T>>()
        });

        let mut ctx = Context::new(
            ConfigBuilder::new()
                .with_flags(T::flags())
                .with_preset(preset)
                .build()
                .unwrap(),
        )
        .unwrap();
        let swz = Swizzle::rgba();

        let data = ctx.compress(&img, swz).unwrap();

        let img2 = ctx.decompress(&data, img.extents, swz).unwrap();

        assert_eq!(img.extents, img2.extents);
        assert_eq!(img.data.len(), img2.data.len());
        for datas in img.data.iter().zip(img2.data.iter()) {
            // Assert that we have a flat u8 array, just to make sure this stops compiling
            // if the inner data is converted to a 2D array.
            let (original_data, decompressed_data): (&Vec<T>, &Vec<T>) = datas;
            assert_eq!(original_data.len(), decompressed_data.len());
        }

        // ASTC being a lossy compression algorithm, we can't compare the data between the two
        // images, but we make sure that the two images are somewhat close.
        assert_lt!(
            img.data
                .iter()
                .flat_map(|frame| frame.chunks(4))
                .zip(img2.data.iter().flat_map(|frame| frame.chunks(4)))
                .map(|(px1, px2)| T::abs_diff(px1, px2) as f64)
                .collect::<Mean>()
                .mean(),
            tolerance
        );
    }

    /// Autogenerate tests for a specific type and image dimensions
    #[macro_export]
    macro_rules! make_compress_tests {
        ($data:ty, $extents:expr, $tolerance:expr) => {
            #[test]
            fn test_fastest() {
                $crate::tests::test_preset_noise::<$data>(
                    $crate::PRESET_FASTEST,
                    $extents,
                    $tolerance,
                );
            }

            #[test]
            fn test_fast() {
                $crate::tests::test_preset_noise::<$data>(
                    $crate::PRESET_FAST,
                    $extents,
                    $tolerance,
                );
            }

            #[test]
            fn test_medium() {
                $crate::tests::test_preset_noise::<$data>(
                    $crate::PRESET_MEDIUM,
                    $extents,
                    $tolerance,
                );
            }

            #[test]
            fn test_thorough() {
                $crate::tests::test_preset_noise::<$data>(
                    $crate::PRESET_THOROUGH,
                    $extents,
                    $tolerance,
                );
            }

            #[test]
            fn test_very_thorough() {
                $crate::tests::test_preset_noise::<$data>(
                    $crate::PRESET_VERY_THOROUGH,
                    $extents,
                    $tolerance,
                );
            }

            #[test]
            fn test_exhaustive() {
                $crate::tests::test_preset_noise::<$data>(
                    $crate::PRESET_EXHAUSTIVE,
                    $extents,
                    $tolerance,
                );
            }
        };
    }

    mod noise {
        use crate::Extents;

        const EXTENTS_3D: Extents = Extents {
            x: 256,
            y: 256,
            z: 4,
        };

        const EXTENTS_2D: Extents = Extents {
            x: 512,
            y: 512,
            z: 1,
        };

        const TOLERANCE_FLOAT: f64 = 0.31;

        mod image_3d {
            use crate::make_compress_tests;
            use crate::Extents;

            const EXTENTS: Extents = super::EXTENTS_3D;
            const TOLERANCE_U8: f64 = 0.615;

            mod type_u8 {
                super::make_compress_tests!(u8, super::EXTENTS, super::TOLERANCE_U8);
            }

            mod type_f32 {
                super::make_compress_tests!(f32, super::EXTENTS, super::super::TOLERANCE_FLOAT);
            }

            mod type_f16 {
                super::make_compress_tests!(
                    half::f16,
                    super::EXTENTS,
                    super::super::TOLERANCE_FLOAT
                );
            }
        }

        mod image_2d {
            use crate::make_compress_tests;
            use crate::Extents;

            const EXTENTS: Extents = super::EXTENTS_2D;
            const TOLERANCE_U8: f64 = 0.315;

            mod type_u8 {
                super::make_compress_tests!(u8, super::EXTENTS, super::TOLERANCE_U8);
            }

            mod type_f32 {
                super::make_compress_tests!(f32, super::EXTENTS, super::super::TOLERANCE_FLOAT);
            }

            mod type_f16 {
                super::make_compress_tests!(
                    half::f16,
                    super::EXTENTS,
                    super::super::TOLERANCE_FLOAT
                );
            }
        }
    }

    mod real_image {
        #[test]
        fn test_fastest() {
            crate::tests::test_preset_image(crate::PRESET_FASTEST, 0.00680);
        }

        #[test]
        fn test_fast() {
            crate::tests::test_preset_image(crate::PRESET_FAST, 0.00665);
        }

        #[test]
        fn test_medium() {
            crate::tests::test_preset_image(crate::PRESET_MEDIUM, 0.00565);
        }

        #[test]
        fn test_thorough() {
            crate::tests::test_preset_image(crate::PRESET_THOROUGH, 0.00540);
        }

        #[test]
        fn test_very_thorough() {
            crate::tests::test_preset_image(crate::PRESET_VERY_THOROUGH, 0.00535);
        }

        #[test]
        fn test_exhaustive() {
            crate::tests::test_preset_image(crate::PRESET_EXHAUSTIVE, 0.00531);
        }
    }
}
