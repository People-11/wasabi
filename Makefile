SHADERC_STAGING := $(CURDIR)/.cargo/shaderc_lib
export SHADERC_LIB_DIR := $(shell cygpath -w "$(SHADERC_STAGING)")
export TOOLCHAIN := nightly-2025-08-15-x86_64-pc-windows-gnu
export RUSTUP_TOOLCHAIN := $(TOOLCHAIN)
export RUSTC_BOOTSTRAP := 1
export RUSTFLAGS := -C target-cpu=x86-64-v3 -C symbol-mangling-version=v0 -C force-frame-pointers=no -C llvm-args=-fp-contract=fast -C link-arg=-Wl,-O1

# egui_winit_vulkano with our patch applied, which Cargo.toml's [patch] points to
EWV_VERSION := 0.28.0
EWV_SHA256 := b6abd5e69939cd416853fc3ec69c0e08721b175d03b508d5574849885c7e85a6
EWV_PATCH := patches/egui_winit_vulkano-$(EWV_VERSION).patch
EWV_DIR := .cargo/egui_winit_vulkano

.PHONY: all release build clean

all: release

release: $(EWV_DIR)/.patched
	mkdir -p "$(SHADERC_STAGING)"
	cp $$(dirname $$(which gcc))/libshaderc_shared.dll "$(SHADERC_STAGING)/shaderc_shared.dll"
	cargo build --release
	rm -rf "$(SHADERC_STAGING)"

build: $(EWV_DIR)/.patched
	mkdir -p "$(SHADERC_STAGING)"
	cp $$(dirname $$(which gcc))/libshaderc_shared.dll "$(SHADERC_STAGING)/shaderc_shared.dll"
	cargo build
	rm -rf "$(SHADERC_STAGING)"

$(EWV_DIR)/.patched: $(EWV_PATCH)
	rm -rf "$(EWV_DIR)"
	mkdir -p "$(EWV_DIR)"
	curl -sSfL -o "$(EWV_DIR).crate" https://static.crates.io/crates/egui_winit_vulkano/egui_winit_vulkano-$(EWV_VERSION).crate
	echo "$(EWV_SHA256)  $(EWV_DIR).crate" | sha256sum -c --quiet
	tar -xzf "$(EWV_DIR).crate" -C "$(EWV_DIR)" --strip-components=1
	rm "$(EWV_DIR).crate"
	git apply --directory="$(EWV_DIR)" $(EWV_PATCH)
	touch $@

clean:
	cargo clean
	rm -rf "$(EWV_DIR)"
