#!/usr/bin/env python3
"""Mutation regressions for the shell appearance source guard."""
import unittest
from check_shell_style import check,inspect_source
class Source(unittest.TestCase):
    def test_tree(self):check()
    def test_private_color(self):self.assertTrue(inspect_source('fill_rect(b,w,h,x,y,a,b,0xff123456);','main.rs'))
    def test_mask_is_not_a_color(self):self.assertEqual(inspect_source('let c=0xff000000 | (theme.panel & 0x00ffffff);','main.rs'),[])
    def test_black_is_still_a_private_color(self):self.assertTrue(inspect_source('fill_rect(b,w,h,x,y,a,b,0xff000000);','main.rs'))
    def test_literal_size(self):self.assertTrue(inspect_source('fill_rrect(b,w,h,x,y,18,h,r,RR_ALL,c);','main.rs'))
    def test_literal_assignment(self):
        self.assertTrue(inspect_source("let padding=8;","shell_runtime.rs"))
        self.assertTrue(inspect_source("let sx=300 + k*70;","protocol_runtime.rs"))
    def test_literal_shade(self):self.assertTrue(inspect_source('shade(theme.border,18);','window_painter.rs'))
    def test_literal_fallback(self):self.assertTrue(inspect_source('let r=(title_h-12).max(4);','window_model.rs'))
    def test_geometry_and_pixel_arithmetic(self):self.assertEqual(inspect_source('fill_rect(b,w,h,x,y,skin.width,1,c); shade(theme.border,skin.gradient);','main.rs'),[])
    def test_comments_tests_strings_excluded(self):self.assertEqual(inspect_source('let s="0xff123456"; // shade(c,18)\n#[cfg(test)] mod tests { const C:u32=0xff123456; }','main.rs'),[])
if __name__=='__main__':unittest.main()
