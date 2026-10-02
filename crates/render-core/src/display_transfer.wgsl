// Linear light to sign-preserving extended sRGB; highlights remain above one.
fn display_encode(rgb: vec3<f32>) -> vec3<f32> {
    let magnitude = abs(rgb);
    let low = magnitude * 12.92;
    let high = 1.055 * pow(magnitude, vec3<f32>(1.0 / 2.4)) - 0.055;
    return sign(rgb) * select(high, low, magnitude <= vec3<f32>(0.0031308));
}
