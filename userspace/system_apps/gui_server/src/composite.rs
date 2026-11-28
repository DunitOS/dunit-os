//! Straight-alpha source-over works for both the opaque desktop and detached
//! transparent animation surfaces.
#[inline(always)]
pub fn blend(dst:u32,src:u32)->u32 {
    let sa=src>>24;
    if sa==0{return dst;}
    if sa==255{return src;}
    let da=dst>>24;
    if da==0{return src;}
    if da==255 {
        let inv = 255 - sa;
        let rb = (src & 0x00FF_00FF) * sa + (dst & 0x00FF_00FF) * inv + 0x0080_0080;
        let g = ((src >> 8) & 255) * sa + ((dst >> 8) & 255) * inv + 128;
        let rb = ((rb + ((rb >> 8) & 0x00FF_00FF)) >> 8) & 0x00FF_00FF;
        let g = (g + (g >> 8)) >> 8;
        return 0xFF00_0000 | rb | (g << 8);
    }
    let weight=sa*255+da*(255-sa);
    let alpha=(weight+127)/255;
    let mut out=alpha<<24;
    for shift in [0,8,16] {
        let s=(src>>shift)&255;let d=(dst>>shift)&255;
        let value=(s*sa*255+d*da*(255-sa)+weight/2)/weight;
        out|=value.min(255)<<shift;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detached_surfaces_keep_straight_alpha_without_black_halo(){
        assert_eq!(blend(0,0x8040a0ff),0x8040a0ff);
        assert_eq!(blend(0x8040a0ff,0),0x8040a0ff);
        let layered=blend(0x800000ff,0x80ff0000);
        assert_eq!(layered>>24,192);
        assert_eq!((layered>>16)&255,170);
        assert_eq!(layered&255,85);
    }
    #[test]
    fn opaque_backdrop_keeps_old_fast_path_endpoints(){
        assert_eq!(blend(0xff203040,0),0xff203040);
        assert_eq!(blend(0xff203040,0xffabcdef),0xffabcdef);
        assert_eq!(blend(0xff000000,0x80ffffff),0xff808080);
    }
    #[test]
    fn opaque_backdrop_fast_path_matches_general_blend() {
        for sa in [1u32, 2, 127, 128, 210, 235, 254] {
            let inv = 255 - sa;
            let weight = sa * 255 + 255 * inv;
            for c in [0u32, 1, 77, 128, 200, 255] {
                for d in [0u32, 15, 128, 240, 255] {
                    let src = (sa << 24) | (c << 16) | ((c ^ 0x55) << 8) | (255 - c);
                    let dst = 0xFF00_0000 | (d << 16) | ((255 - d) << 8) | (d ^ 0x33);
                    let mut ref_px = 0xFF00_0000u32;
                    for shift in [0, 8, 16] {
                        let sc = (src >> shift) & 255;
                        let dc = (dst >> shift) & 255;
                        let v = (sc * sa * 255 + dc * 255 * inv + weight / 2) / weight;
                        ref_px |= v.min(255) << shift;
                    }
                    assert_eq!(blend(dst, src), ref_px);
                }
            }
        }
    }
}
