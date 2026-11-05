/* Seal the declared C roots into compact sections for ordinary C linkers.
 * Input sections are garbage-collected before coalescing; no runtime code or
 * symbol addresses are fixed here. */
SECTIONS {
  .text : { *(.text .text.*) }
  .rodata : { *(.rodata .rodata.*) }
  .data.rel.ro : { *(.data.rel.ro .data.rel.ro.*) }
  .data : { *(.data .data.*) }
  .bss : { *(.bss .bss.*) *(COMMON) }
}
