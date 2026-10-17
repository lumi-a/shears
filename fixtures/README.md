# Fixtures

Compressed with tar+zstd:
```sh
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'zstd --ultra -22 -T0' -cf vanilla-1.20.1.tar.zst vanilla-1.20.1
```

## vanilla-1.20.1.tar.zst

Exported with `/kubejs export` ingame. Had to put `ServerEvents.recipes(event => {})` into the kubejs files for the exporting to dump the recipes too.

- Minecraft 1.20.1, Forge 47.4.10
- KubeJS 2001.6.5-build.26, Rhino 2001.2.3-build.10
- Architectury 9.2.14
