fn main() {
    // Keep the regression paths in the linked image as well as the object check.
    std::hint::black_box(compiler_reproducibility::resize_f32);
    std::hint::black_box(compiler_reproducibility::resize_f32x2);
    std::hint::black_box(compiler_reproducibility::resize_f32x3);
    std::hint::black_box(compiler_reproducibility::resize_f32x4);
}
