/* spin.c — EL0 test program for O5.2 preemption: loops forever and never
 * makes a syscall, so only the timer IRQ can take the CPU back from it.
 *
 * Build: tools/build_test_programs.sh
 */

void _start(void) {
    volatile unsigned long counter = 0;
    for (;;) {
        counter++;
    }
}
