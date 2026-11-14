package net.minecraft.core;

/** Immutable coordinates supplied by the native before-build edit snapshot. */
public class Vec3i {
    private final int x;
    private final int y;
    private final int z;

    protected Vec3i(int x, int y, int z) { this.x = x; this.y = y; this.z = z; }
    public final int getX() { return x; }
    public final int getY() { return y; }
    public final int getZ() { return z; }
}
