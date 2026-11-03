package net.minecraft.core;

/** Immutable native edit position; no world lookup or mutable-position facade. */
public final class BlockPos extends Vec3i {
    public BlockPos(int x, int y, int z) { super(x, y, z); }
}
