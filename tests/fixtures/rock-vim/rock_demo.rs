/* Rock 2.0 Showcase: Hybrid Nano + Vim */
#[inline]
pub fn compute(limit: usize) -> Option<u32> {
    let banner = "Dunit Rock 2.0";
    let mut acc: u32 = 0x10;
    // Iterate with matching brackets & operators
    for idx in 0..limit {
        if (idx % 2) == 0 && !banner.is_empty() {
            acc = acc.wrapping_add(idx as u32);
        }
    }
    if acc > 42 { Some(acc) } else { None }
}
