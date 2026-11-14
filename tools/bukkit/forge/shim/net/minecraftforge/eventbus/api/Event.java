package net.minecraftforge.eventbus.api;

/** Cancellation state shared by the one supported BreakEvent callback. */
public abstract class Event {
    private boolean canceled;

    public abstract boolean isCancelable();
    public final boolean isCanceled() { return canceled; }
    public final void setCanceled(boolean value) {
        if (!isCancelable()) throw new IllegalArgumentException("event is not cancelable");
        canceled = value;
    }
}
