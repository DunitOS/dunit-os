# Dunit release images

Every new release contains two ISO files:

| Image | Boot mode | Included software |
| --- | --- | --- |
| `dunit_dwm.iso` | DWM desktop | GUI apps, Settings, desktop profiles, Lua, native C toolchain and base utilities |
| `dunit_minimal.iso` | Terminal | `dsh`, `dufetch`, `calc`, `fsck_dunit` and the command search path |

Both images use the same kernel, native ABI and DUNITRD1 archive format. Minimal
omits GUI executables, desktop services, themes, icons, fonts, wallpapers, Lua
and compiler payloads. It does not remove kernel mechanisms or change the ABI.

Both retain the live installer and BIOS/UEFI boot support. Minimal uses a 64 MiB
FAT32 installer payload instead of DWM's 128 MiB payload. The in-OS installer
already derives the ESP partition size from the payload length. This preserves
the existing filesystem format and installation path. The installer volume and
kernel account for most of the Minimal ISO size, not its small userspace archive.

## Build

Use the normal Linux build dependencies described in the README. The image
verification tools additionally require Python 3.11 or later (`tomllib`),
`xorriso` and `mtools`. UEFI tests use OVMF.

```bash
./build_iso.sh       # both release images
make iso-release    # both, after dependencies/submodules are available
make iso-dwm        # build/dunit_dwm.iso
make iso-minimal    # build/dunit_minimal.iso
```

Release builds run sequentially because kernel outputs and staging directories
are shared. Each ISO gets a fresh staging directory. Existing files in
`build/userspace` cannot leak into a release archive: the packer requires the
applications listed in the selected profile and rejects missing ones.

Composition is declared in [Minimal](../images/minimal/profile.json) and
[DWM](../images/dwm/profile.json) manifests. They select Rust and C applications,
assets, services, toolchain inclusion, boot configuration and installer volume
size. Each archive records its profile in `/system/share/dunit-image.json`.
DWM includes the compositor's current shared-buffer helper and all applications
registered in the default desktop configuration.

The internal `full` profile preserves the existing developer test payload and
smoke fixtures. `make iso LIMINE_CONFIG=limine_test_terminal.conf` still writes
`build/microkernel.iso` for those tests. This internal image is not a third
release edition and is not uploaded as a release asset.

For disk images, the installer accepts the same profile:

```bash
python3 tools/install_disk.py build/minimal-disk.img --profile minimal \
  --yes-i-know-this-erases-the-disk
```

The example target is a regular image file. Real disk installation remains an
explicit destructive operation.

## Verify

Build and boot either release through the standard harness:

```bash
python3 tools/qemu_test.py --build --variant minimal \
  --markers-file tools/minimal_markers.json --timeout 240

python3 tools/qemu_test.py --build --variant dwm \
  --markers-file tools/dwm_release_markers.json \
  --ready-marker 'gui_server: desktop ready' --linger 3 --timeout 300
```

`--variant` selects its own profile and boot configuration. Existing `--build
--config ...` tests keep their full developer payload. To test a previously
built release, use `--iso build/dunit_minimal.iso` or `--iso build/dunit_dwm.iso`.
Add `--uefi` to check its firmware path.

Check actual ISO contents, including the embedded installer's initrd:

```bash
python3 tools/test_image_profiles.py
python3 tools/check_image_profile.py build/dunit_minimal.iso minimal
python3 tools/check_image_profile.py build/dunit_dwm.iso dwm
```

The checker validates exact application and service sets, boot configuration,
profile metadata, desktop registry executables and required runtime tools.
It also extracts the installer's FAT image and compares its initrd byte for
byte with the live initrd. Minimal rejects every extra payload file.

Local validation for this change passed seven host tests, composition checks
for both ISOs, and BIOS and UEFI boots of both release variants. Minimal executed
`dufetch` successfully. The existing production Settings scenario checks dock
launch, palette scrolling, Lua activation and layout switching on the DWM ISO.
Both release variants also passed BIOS boots under TCG. The existing full
boot-marker regression passed with `runtime_stress` and IPC, and the compiler
warning check measured 13 warnings against the existing budget of 99.

Image sizes on this build were approximately 75 MiB for Minimal and 240 MiB for
DWM; sizes will change as the kernel and packaged software evolve.

## Release publication

The GitHub workflow builds both editions and checks their contents, BIOS/UEFI
boot behavior and the existing compiler-warning and boot-marker budgets before
creating a release draft. It uploads only `dunit_dwm.iso` and
`dunit_minimal.iso`, verifies that both named assets are present, then publishes
the draft. A failed build or test prevents publication; a failed/incomplete
upload leaves the release unpublished. Tags point to the exact tested commit.
