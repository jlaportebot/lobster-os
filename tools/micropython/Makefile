# LobsterOS MicroPython port — minimal build
MICROPYTHON_SRC = src
LIBC_DIR = ../libc
BUILD = build

CC = aarch64-linux-gnu-gcc

CFLAGS = -ffreestanding -nostdlib -static -Os \
         -fdata-sections -ffunction-sections \
         -include mpconfigport.h \
         -I. \
         -I$(MICROPYTHON_SRC) \
         -I$(MICROPYTHON_SRC)/py \
         -I$(LIBC_DIR)/include \
         -I$(BUILD)

LDFLAGS = -nostdlib -static -nostartfiles \
          -Wl,-Ttext=0x10000000 \
          -Wl,--build-id=none \
          -Wl,--nmagic \
          -Wl,--gc-sections

LIBS = -L$(LIBC_DIR) -llobsterc

# Py core objects
PY_O = $(addprefix $(BUILD)/py/, \
    mpstate.o nlr.o nlraarch64.o malloc.o gc.o pystack.o qstr.o vstr.o \
    mpprint.o unicode.o mpz.o reader.o lexer.o parse.o scope.o compile.o \
    emitcommon.o emitbc.o emitndebug.o formatfloat.o parsenumbase.o parsenum.o \
    emitglue.o persistentcode.o runtime.o runtime_utils.o scheduler.o \
    nativeglue.o pairheap.o ringbuf.o cstack.o stackctrl.o argcheck.o \
    warning.o map.o obj.o objarray.o objattrtuple.o objbool.o objboundmeth.o \
    objcell.o objclosure.o objcode.o objcomplex.o objdeque.o objdict.o \
    objenumerate.o objexcept.o objfilter.o objfloat.o objfun.o objgenerator.o \
    objgetitemiter.o objint.o objint_longlong.o objint_mpz.o objlist.o \
    objmap.o objmodule.o objobject.o objpolyiter.o objproperty.o objnone.o \
    objnamedtuple.o objrange.o objreversed.o objringio.o objset.o \
    objsingleton.o objslice.o objstr.o objstrunicode.o objstringio.o \
    objtemplate.o objtuple.o objtype.o objzip.o opmethods.o sequence.o \
    stream.o binary.o builtinimport.o builtinevex.o modarray.o modbuiltins.o \
    modcollections.o modgc.o modio.o modmath.o modcmath.o modmicropython.o \
    modstring.o modstruct.o modsys.o moderrno.o modweakref.o vm.o bc.o repl.o smallint.o \
    frozenmod.o)

# Shared/port objects
PORT_O = \
    $(BUILD)/main.o \
    $(BUILD)/uart_core.o \
    $(BUILD)/shared/libc/printf.o \
    $(BUILD)/shared/libc/string0.o \
    $(BUILD)/shared/readline/readline.o \
    $(BUILD)/shared/runtime/pyexec.o \
    $(BUILD)/shared/runtime/stdout_helpers.o

ALL_O = $(PY_O) $(PORT_O)

# Generic compile rule for py/ files
$(BUILD)/py/%.o: $(MICROPYTHON_SRC)/py/%.c $(BUILD)/genhdr/qstrdefs.generated.h
	@mkdir -p $(BUILD)/py
	$(CC) $(CFLAGS) -c $< -o $@

# Generic compile rule for shared/ files
$(BUILD)/shared/%.o: $(MICROPYTHON_SRC)/shared/%.c $(BUILD)/genhdr/qstrdefs.generated.h
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

# Compile rule for port files
$(BUILD)/%.o: %.c $(BUILD)/genhdr/qstrdefs.generated.h
	@mkdir -p $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

# Generate qstr definitions
$(BUILD)/genhdr/qstrdefs.generated.h: $(MICROPYTHON_SRC)/py/qstrdefs.h
	@mkdir -p $(BUILD)/genhdr
	@python3 scripts/genqstr.py $(MICROPYTHON_SRC) $(BUILD)/genhdr

# Link final ELF
$(BUILD)/micropython.elf: $(ALL_O) $(LIBC_DIR)/liblobsterc.a
	$(CC) $(LDFLAGS) -o $@ $(ALL_O) $(LIBS)
	@echo "Built: $@ ($$(stat -c%s $$@ 2>/dev/null || echo ?) bytes)"

all: $(BUILD)/micropython.elf
clean:
	rm -rf $(BUILD)
.PHONY: all clean
