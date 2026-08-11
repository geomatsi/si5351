MEMORY
{
    /* STM32F103C8T6 Blue Pill: 64K flash, 20K RAM. Boards sold as C8 often
       carry a CB die with 128K; raise LENGTH if yours does. */
    FLASH : ORIGIN = 0x08000000, LENGTH = 64K
    RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}
