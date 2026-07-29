# Presets

What SqueezeIt is allowed to do to each kind of asset, and why.

Every rule here came out of breaking a real clothing pack on a real AMD machine. The
short version: **vehicles and props can take everything, clothing can only be resized,
hair should mostly be left alone.**

## How a file gets classified

Two things are worked out separately.

**The family** is per file. In order of trust:

1. Shader hash. `ped_hair_cutout_alpha`, `ped_hair_spiked` and `ped_fur` mean hair.
   `vehicle_*` means vehicle. `ped_*` and `cloth_*` mean clothing.
2. A dictionary holding a `vehicle_generic*` texture means vehicle.
3. The filename. Ped drawables are named `<pedmodel>^<component>_<index>[_<variant>]`,
   so `mp_f_freemode_01^hair_006_u.ydd` is hair and `^uppr_012_r.ydd` is clothing.
   `hair` and `berd` are hair, everything else in the component list is clothing.
4. Anything left over is generic.

Shaders beat filenames, because a renamed file is common and a wrong shader hash is not.
Hair is the one exception: a file called `^hair_006_u.ydd` is treated as hair even when it
declares a cloth shader, because that is what those drawables actually are. `.ytd` files
have no shader group at all, so the name is all we get.

**The role** is per texture, from its name inside the dictionary, with a pixel-content
fallback for unlabelled maps. Roles are diffuse, normal, specular, hair, livery, weapon
and `script_rt`.

A single texture that reads as hair inside an otherwise normal dictionary gets the hair
treatment on its own.

## The presets

`Auto` is the default and dispatches per file. The named presets force one row onto
everything, for folders you already know are one kind of asset.

|                  | Clothing        | Hair & alpha       | Vehicles & props |
| ---------------- | --------------- | ------------------ | ---------------- |
| Diffuse cap      | 1024            | 1024, floor 512    | your limit       |
| Normal/spec cap  | 512             | 1024               | 1024             |
| Generate mips    | no (opt-in)     | **never**          | yes for props, opt-in for vehicles |
| Trim the mip tail| yes             | **no**             | yes              |
| Format changes   | never BC1       | **locked**         | aggressive       |
| Overdrive        | forced off      | forced off         | allowed          |
| GPU              | off (opt-in)    | forced off         | allowed          |
| Encoder floor    | Normal          | Normal             | your choice      |

Caps are ceilings, not targets. The effective limit is always
`min(your Resolution Limit, the preset cap)`, so lowering the limit still works
everywhere and raising it only lifts families that have no ceiling of their own.

`Hair (strict)` is `Hair` with resizing off too. Files are read, checked, and written back
unchanged. Use it when a pack is already tuned and you only want the rest of the folder
touched.

`Custom` turns the family rules off entirely and runs on the raw switches. It is the only
way to do something the presets consider unsafe, and it is on you if it breaks.

## Always protected

Under every preset, including Custom:

- `script_rt*` textures are never touched. They are live render targets.
- Liveries, `_sign_` textures and weapon skins are never downscaled. They are read
  close-up and at high zoom.
- Normal maps never get downgraded to BC1.
- A container is never written if its header would promise more bytes than the graphics
  segment actually holds. That check runs on every path.

## Why hair and cloth are locked down

Three failures, all reproducible on the same AMD test machine:

- **Hair strands broke up at mid distance** after mipmap generation. Mips alone were
  enough. No format change needed.
- **Shoe colours shifted** with generated mips on cloth, and again with Overdrive, which
  puts a half-size normal map against a full-size diffuse.
- **`amdxx64.dll` crashed in `CreateTexture2D` during load** when generated mips and a
  format change landed on the same clothing texture.

The cause was never proven. What is known is that the crashes correlate with the
combination of alpha, a format change, and a rebuilt mip chain hitting one texture at
once. The policy avoids the whole combination rather than pretending to know which part
is at fault.

BC1 is banned on anything ped-related for a separate reason: its 5:6:5 endpoints and
1-bit alpha wreck skin tones, fabric gradients and cutout edges. That one is not a guess.

## The three opt-ins

The UI only shows each of these when the current preset actually reaches that family.

**Cloth mip chains.** Lets one clothing texture grow a chain, but only when *all* of
these hold:

- no alpha anywhere in the top mip
- the output format matches the source format
- it shipped with no chain at all (`levels <= 1`)
- it is square, a power of two, and at least 128 wide
- it is in a `.ytd` (drawables are patched in place and cannot grow)

Every one of those is a factor that co-occurred with the crashes. Requiring all of them
to be absent is the conservative reading. Anything that fails a condition falls back to
trimming the tail.

**Cloth on GPU.** Lets clothing be block-compressed on the graphics card. Off because GPU
encodes on cloth lined up with driver resets and corrupt drawables.

**Vehicle mip chains.** Lets vehicle textures grow full chains. Vehicles have been
excluded from mip generation since the first release and nobody wrote down why, so the
opt-in exists to settle it with a test rather than a guess.

## Mip levels

```
full     = rage_mip_levels(w, h)          // ilog2(long side) - 1
physical = physical_mip_levels(w, h)      // ilog2(long side) + 1

Preserve     -> source levels, clamped to physical
TrimTail     -> source levels, clamped to full
GenerateFull -> full, but only in a .ytd; drawables clamp to source levels
```

RAGE never samples the 2×2 and 1×1 tail, which is why `full` stops one short. The count is
keyed off the **long** side, so a 512×32 texture keeps the chain its 512 axis needs.

`Preserve` clamps to `physical` rather than `full` so a preserved chain carried onto a
smaller output does not overrun, while an unresized hair texture keeps its levels exactly
and passes straight through the "already optimal" early-out.

## Drawables don't shrink structurally

`.ydd`, `.ydr` and `.yft` keep their original graphics segment length. Textures are
patched into their existing slot and the tail is zero-filled. The file still gets smaller,
because zeros deflate to nothing — but the segment layout never moves. This is why cloth
resizing pays off even though the container size looks fixed, and why drawables can never
grow a mip chain.

`.ytd` containers are rebuilt properly, so they genuinely shrink. Identical texture
payloads inside one dictionary are folded onto a single offset while we are in there.

## Known gaps

- The report counts files, not families. It cannot yet tell you "412 clothing textures
  resized, 0 mips generated". The per-file `tracing` output has it if you need it.
- Alpha-test coverage preservation on downscale is a known technique for alpha-tested
  geometry and would directly counter hair strand thinning. Not implemented.
- The 512 hair floor is a chosen number, not a measured threshold.
- `berd` is treated as hair. If a pack puts opaque masks under `^berd`, they get optimized
  more carefully than they need to be. That costs savings, not safety.

## The backup vault

`--backup` moves each original into `.squeezeit-backup/` under the folder you pointed at,
records it in `squeezeit-manifest.tsv`, and writes the optimized file in its place.
`--restore` replays that manifest backwards and deletes the vault.

Every stored file gets a `.sqzbak` suffix appended after its real extension. This is not
cosmetic. The vault frequently ends up inside a resource's `stream/` directory, FiveM
enumerates that directory recursively and claims files by extension, and a dot-prefixed
folder name does not exempt it. Without the suffix the engine sees two assets with the
same internal name and picks one arbitrarily, which looks exactly like the optimization
having done nothing.

`collect_targets` also skips the vault directory outright, so a second run never
re-processes originals.

Restore accepts both suffixed and un-suffixed vault entries, so vaults written by older
builds still roll back.
