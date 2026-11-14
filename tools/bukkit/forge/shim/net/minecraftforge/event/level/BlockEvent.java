package net.minecraftforge.event.level;

import net.minecraft.core.BlockPos;
import net.minecraftforge.eventbus.api.Event;

/** Native before-build position snapshot. No Level, state, player or XP API. */
public abstract class BlockEvent extends Event {
    private final BlockPos position;

    private BlockEvent(BlockPos position) {
        this.position = position;
    }

    public final BlockPos getPos() { return position; }

    public static final class BreakEvent extends BlockEvent {
        /** Converter-only constructor; original mod calls to constructors are refused. */
        public BreakEvent(BlockPos position) { super(position); }
        @Override public boolean isCancelable() { return true; }
    }
}
