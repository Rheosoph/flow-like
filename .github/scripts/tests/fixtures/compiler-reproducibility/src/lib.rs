use fast_image_resize::{
    images::{TypedImage, TypedImageRef},
    pixels::{F32x2, F32x3, F32x4, F32},
    ResizeOptions, Resizer,
};

// These F32 convolution paths exposed LLVM's unordered loop-unroll traversal.
// Keep every variant instantiated with release optimizations enabled.
// Upstream fix: https://github.com/llvm/llvm-project/pull/188821
macro_rules! resize {
    ($name:ident, $pixel:ty) => {
        #[inline(never)]
        pub fn $name(
            src: &TypedImageRef<$pixel>,
            dst: &mut TypedImage<$pixel>,
            resizer: &mut Resizer,
        ) {
            resizer
                .resize_typed(src, dst, &ResizeOptions::default())
                .unwrap();
        }
    };
}

resize!(resize_f32, F32);
resize!(resize_f32x2, F32x2);
resize!(resize_f32x3, F32x3);
resize!(resize_f32x4, F32x4);
