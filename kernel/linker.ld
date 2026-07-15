ENTRY(_start)

SECTIONS
{
    . = 32M;

    .text : ALIGN(4K) {
        *(.text .text.*)
    }

    .usertext : ALIGN(4K) {
        KEEP(*(.usertext))
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        *(COMMON)
        *(.bss .bss.*)
    }

    .userstack (NOLOAD) : ALIGN(4K) {
        KEEP(*(.userstack))
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.note .note.*)
    }
}
