//! Straight-alpha source-over works for both the opaque desktop and detached
//! transparent animation surfaces.
pub fn blend(dst:u32,src:u32)->u32 {
    let sa=src>>24;
    if sa==0{return dst;}
    if sa==255{return src;}
    let da=dst>>24;
    if da==0{return src;}
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
}
