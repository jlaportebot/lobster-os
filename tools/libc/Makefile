CC = aarch64-linux-gnu-gcc
CFLAGS = -ffreestanding -nostdlib -static -O2 -Iinclude
AR = aarch64-linux-gnu-ar
ARFLAGS = rcs

SRC = stdio.c string.c stdlib.c unistd.c fcntl.c errno.c start.c assert.c math.c
OBJ = $(SRC:.c=.o)

TARGET = liblobsterc.a

all: $(TARGET)

$(TARGET): $(OBJ)
	$(AR) $(ARFLAGS) $@ $^

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJ) $(TARGET)

.PHONY: all clean