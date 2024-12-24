# executable name
output := wayland-osd-wireplumber-monitor
# source files
srcdir := src
srcs := main.c lib/log.c

# compiler flags
CFLAGS += -Wall -Wextra -pedantic -Wshadow -Wconversion -Wdouble-promotion -Wformat=2 -Werror -Os -g -std=c23 -DLOG_USE_COLOR
# linker flags
#LDFLAGS+= -lX11 -llua5.3
pkgs := wireplumber-0.5

######################################################################

include .Nice.mk
