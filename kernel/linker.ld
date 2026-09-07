ENTRY(_start)

SECTIONS
{
    . = 32M;

    .text : ALIGN(4K) {
        __kernel_text_start = .;
        *(.text .text.*)
        *(.ltext .ltext.*)
        . = ALIGN(4K);
        __kernel_text_end = .;
    }

    .rodata : ALIGN(4K) {
        __kernel_rodata_start = .;
        *(.rodata .rodata.*)
        *(.lrodata .lrodata.*)
        . = ALIGN(4K);
        __kernel_rodata_end = .;
    }

    .data : ALIGN(4K) {
        __kernel_data_start = .;
        *(.data .data.*)
        *(.ldata .ldata.*)
        *(.got .got.*)
    }

    .bss : ALIGN(4K) {
        *(COMMON)
        *(.bss .bss.*)
        *(.lbss .lbss.*)
        . = ALIGN(4K);
        __kernel_data_end = .;
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.note .note.*)
    }
}
