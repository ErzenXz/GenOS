ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5);
    data PT_LOAD FLAGS(6);
}

SECTIONS
{
    . = 0x0000400000001000;
    .text : ALIGN(4K) {
        *(.text._start)
        *(.text .text.*)
        *(.ltext .ltext.*)
        *(.rodata .rodata.*)
        *(.lrodata .lrodata.*)
    } :text
    ASSERT(SIZEOF(.text) <= 4K, "GenOS init text exceeds one page")

    . = 0x0000400000002000;
    .data : ALIGN(4K) {
        *(.data.process)
        *(.data .data.*)
        *(.ldata .ldata.*)
        *(.got .got.*)
        *(.bss .bss.*)
        *(.lbss .lbss.*)
        *(COMMON)
    } :data
    ASSERT(SIZEOF(.data) <= 4K, "GenOS init data exceeds one page")

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
    }
}
