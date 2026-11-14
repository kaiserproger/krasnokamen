package net.minecraftforge.eventbus.api;

/** Only NORMAL is accepted by the frontend; other values are not dispatched. */
public enum EventPriority { HIGHEST, HIGH, NORMAL, LOW, LOWEST, MONITOR }
