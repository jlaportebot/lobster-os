// LobsterOS MicroPython port config
#ifndef MPCONFIGPORT_H
#define MPCONFIGPORT_H

#include <stdint.h>

// Use minimum ROM configuration
#define MICROPY_CONFIG_ROM_LEVEL (0)

// Needed for integer types — let mpconfig.h auto-detect from INTPTR_MAX
// AArch64: intptr_t is 64-bit, so INTPTR_MAX = LLONG_MAX
// Don't redefine MP_INT_MAX — let mpconfig.h handle it

// Enable modules
#define MICROPY_ENABLE_FINALISER    (1)
#define MICROPY_PY_THREAD           (0)  // Disable threading — no thread support
#define MICROPY_PY_BUILTINS_FLOAT   (1)  // Required by math/cmath modules
#define MICROPY_PY_BUILTINS_COMPLEX (1)  // Required by cmath

// Float type — use double on AArch64
#define MICROPY_FLOAT_IMPL (MICROPY_FLOAT_IMPL_FLOAT)
typedef float mp_float_t;
#define MICROPY_PY_CMATH            (1)
#define MICROPY_PY_GC               (1)
#define MICROPY_PY_MATH             (1)
#define MICROPY_PY_MICROPYTHON     (1)
#define MICROPY_PY_WEAKREF          (1)

// System mutable slots (sys.modules, sys.path, etc.)
#define MP_SYS_MUTABLE_NUM 1
#define MICROPY_SCHEDULER_DEPTH 4
#define MICROPY_ENABLE_COMPILER     (1)

// GC for heap allocation
#define MICROPY_ENABLE_GC           (1)
#define MICROPY_HELPER_REPL         (1)

// Path
#define MICROPY_ALLOC_PATH_MAX      (256)

// Minimal allocation
#define MICROPY_ALLOC_PARSE_CHUNK_INIT (16)

// Disable optional sys features
#define MICROPY_PY_SYS_MODULES      (0)
#define MICROPY_PY_SYS_EXIT         (0)
#define MICROPY_PY_SYS_PATH         (0)
#define MICROPY_PY_SYS_ARGV         (0)

// Use AArch64 NLR (non-local return via inline assembly)
#define MICROPY_NLR_AARCH64         (1)
#define MICROPY_NLR_SETJMP          (0)

// Use stdout for I/O
#define MICROPY_MIN_USE_STDOUT      (1)
#define MICROPY_HEAP_SIZE           (65536)  // 64KB heap

// Board name
#define MICROPY_HW_BOARD_NAME "LobsterOS"
#define MICROPY_HW_MCU_NAME   "AArch64-QEMU"

// Type definitions
typedef long mp_off_t;

// We provide our own alloca via our libc
#define alloca(size) __builtin_alloca(size)

// No frozen modules
#define MICROPY_MODULE_FROZEN_MPY   (0)
#define MICROPY_ENABLE_EXTERNAL_IMPORT (0)

#define MP_STATE_PORT MP_STATE_VM

// Disable features we don't support
#define MICROPY_PY_IO               (0)
#define MICROPY_PY_IO_FILEIO        (0)
#define MICROPY_PY_IO_BYTESIO        (1)
#define MICROPY_PY_IO_BUFFEREDREADER (0)

#endif // MPCONFIGPORT_H
