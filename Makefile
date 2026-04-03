SHADERC_STAGING := $(CURDIR)/.cargo/shaderc_lib
export SHADERC_LIB_DIR := $(SHADERC_STAGING)
export TOOLCHAIN := nightly-2025-08-15-x86_64-pc-windows-gnu
export RUSTUP_TOOLCHAIN := $(TOOLCHAIN)
export RUSTC_BOOTSTRAP := 1
export RUSTFLAGS := -C target-cpu=x86-64-v3 -C symbol-mangling-version=v0 -C force-frame-pointers=no -C llvm-args=-fp-contract=fast -C link-arg=-Wl,-O1

.PHONY: all release build clean

all: release

release:
	@powershell -Command "New-Item -ItemType Directory -Force -Path '$(SHADERC_STAGING)' | Out-Null; Copy-Item -Force (Join-Path (Split-Path (Get-Command x86_64-w64-mingw32-gcc).Source) 'libshaderc_shared.dll') '$(SHADERC_STAGING)\shaderc_shared.dll'"
	cargo build --release
	@powershell -Command "Remove-Item -Recurse -Force '$(SHADERC_STAGING)'"

build:
	@powershell -Command "New-Item -ItemType Directory -Force -Path '$(SHADERC_STAGING)' | Out-Null; Copy-Item -Force (Join-Path (Split-Path (Get-Command x86_64-w64-mingw32-gcc).Source) 'libshaderc_shared.dll') '$(SHADERC_STAGING)\shaderc_shared.dll'"
	cargo build
	@powershell -Command "Remove-Item -Recurse -Force '$(SHADERC_STAGING)'"

clean:
	cargo clean
