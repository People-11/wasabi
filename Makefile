SHADERC_STAGING := $(CURDIR)/.cargo/shaderc_lib
export SHADERC_LIB_DIR := $(shell cygpath -w "$(SHADERC_STAGING)")
export TOOLCHAIN := nightly-2025-08-15-x86_64-pc-windows-gnu
export RUSTUP_TOOLCHAIN := $(TOOLCHAIN)
export RUSTC_BOOTSTRAP := 1
export RUSTFLAGS := -C target-cpu=x86-64-v3 -C symbol-mangling-version=v0 -C force-frame-pointers=no -C llvm-args=-fp-contract=fast -C link-arg=-Wl,-O1

.PHONY: all release build clean

all: release

release:
	mkdir -p "$(SHADERC_STAGING)"
	cp $$(dirname $$(which gcc))/libshaderc_shared.dll "$(SHADERC_STAGING)/shaderc_shared.dll"
	cargo build --release
	rm -rf "$(SHADERC_STAGING)"

build:
	mkdir -p "$(SHADERC_STAGING)"
	cp $$(dirname $$(which gcc))/libshaderc_shared.dll "$(SHADERC_STAGING)/shaderc_shared.dll"
	cargo build
	rm -rf "$(SHADERC_STAGING)"

clean:
	cargo clean
