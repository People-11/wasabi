<h1 align="center">Wasabi</h1>
<p align="center"><img src="/assets/logo.svg" width="128"/></p>
<p align="center">Wasabi is a modern and fast real-time MIDI player written in Rust.</p>
<p align="center">
<img alt="GitHub License" src="https://img.shields.io/github/license/BlackMIDIDevs/wasabi">
<img alt="GitHub Release" src="https://img.shields.io/github/v/release/BlackMIDIDevs/wasabi">
<img alt="GitHub Downloads (all assets, all releases)" src="https://img.shields.io/github/downloads/BlackMIDIDevs/wasabi/total">
</p>

## Features

- Extremely fast and optimized rendering using Vulkan
- Easy to use and configurable
- Integrated MIDI synthesizer (XSynth), alongside with KDMAPI and MIDI device support
- Partial support for Zenith color palettes

## Installation

Wasabi is a portable application and does not require an installation.
Your system must support Vulkan.

### Option A *(recommended)*

You can download and run a pre-built binary of Wasabi from the [releases page.](https://github.com/BlackMIDIDevs/wasabi/releases)

### Option B *(advanced)*

You can build Wasabi yourself by following these steps:

- Clone the repository using `git clone https://github.com/BlackMIDIDevs/wasabi.git` (or [download as a ZIP from GitHub](https://github.com/BlackMIDIDevs/wasabi/archive/refs/heads/master.zip))
- Required tools:
  - [Rust toolchain](https://rustup.rs/)
  - [Vulkan SDK](https://vulkan.lunarg.com/)
  - [CMake **3.X**](https://cmake.org/)
  - [Ninja](https://ninja-build.org/)
  - (C++ build tools for-)[Visual Studio 17+](https://visualstudio.microsoft.com/) (Windows only)
- Inside the project directory run the following command to build Wasabi: `cargo build --release`
  - Optionally you can add `RUSTFLAGS="-C target-cpu=native"` to your environment before compiling to optimize XSynth for your specific CPU
- After the compilation is finished, you will find the binary under `./target/release`

### Option C *(MSYS2, no MSVC required)*

If you prefer to build without MSVC or the Vulkan SDK, you can use [MSYS2](https://www.msys2.org/) with the MinGW-w64 toolchain instead.

1. Install MSYS2 and open the **MINGW64** shell.
2. Install the required packages:
   
   ```bash
   pacman -S mingw-w64-x86_64-rust \
             mingw-w64-x86_64-gcc \
             mingw-w64-x86_64-cmake \
             mingw-w64-x86_64-ninja \
             mingw-w64-x86_64-shaderc \
             mingw-w64-x86_64-pkgconf \
             make
   ```
3. Create a `make` alias (only needed if `make` is not found):
   
   ```bash
   ln -s /mingw64/bin/mingw32-make.exe /mingw64/bin/make.exe
   ```
4. Clone the repository and build using the provided Makefile:
   
   ```bash
   git clone https://github.com/BlackMIDIDevs/wasabi.git
   cd wasabi
   make release
   ```

- After the compilation is finished, you will find the binary under `./target/release`

## Usage

- Before you can play a MIDI, you need to add soundfonts to the synthesizer by going to `Menu -> Settings -> SoundFonts`
- To open a MIDI, click the folder icon on the top left, or press `Ctrl+O` on your keyboard
- To find out about other keyboard shortcuts, head to `Menu -> Shortcuts`

## Screenshot

<p align="center"><img src="/assets/screenshot.png"/></p>

## License

Wasabi is licensed under the [GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0.en.html#license-text).

## Performance

This fork includes several performance optimizations. However, when VSYNC is enabled, these improvements may not be fully realized, and you may observe that the video renderer is not running at full speed.

This is because Vulkan typically uses `VK_PRESENT_MODE_FIFO_KHR` when VSYNC is enabled. On Windows with NVIDIA drivers, this path is often implemented via a layered DXGI swapchain, introducing additional composition and synchronization overhead that can limit overall performance.

By setting **Vulkan/OpenGL Present Method** to **Prefer Native** in the NVIDIA Control Panel, the driver will avoid the DXGI intermediary and use a more direct, native presentation path. This can significantly improve performance and allow the renderer to reach its full potential.

However, note that this setting may break compatibility with applications that rely on DXGI-based hooking, such as "Game Capture" in OBS.

## Crash

Like someone who keeps insisting that “you need a dGPU to run Wasabi because Intel drivers are shit and don’t support advanced Vulkan features”, I regret to say this is probably partly true—Wasabi does sacrifice quite of compatibility for performance.

However, unlike his arrogance, I fixed one issue (the only one I personally encountered): when using Cake on an iGPU, a crash can occur because 256 bindings exceed the `maxPerStageDescriptorStorageBuffers` limit. So if you’re using Cake on an iGPU and hit this crash, you can try switching to Pie.

I can’t help with other crashes (especially Wasabi doesn’t even start).
