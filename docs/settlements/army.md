# Army

Native resident soldiers defend their settlement under player orders.

See [construction](construction.md) for barracks, [menus](menus.md) for dialogs, and [jobs](jobs.md) for residents and work.

## Recruitment and equipment

A certified barracks provides 2, 4, or 6 recruit slots at level 1, 2, or 3 respectively. A recruit is an existing idle adult resident with no profession or job site; residents reserved for production or caravans are ineligible. Recruits start on patrol.

Unowned settlements attempt automatic recruitment only when their anchor chunk is active: at most one attempt per 100 ticks across all unowned settlements. Owned settlements recruit explicitly through the Army dialog or command, so a dismissed soldier stays dismissed until recruited again. Recruitment and combat do not load inactive chunks.

Recruitment fills empty equipment slots from settlement storage, selecting the highest-ranked available sword or axe and armor for each slot. Existing equipment is retained. Gear is real stock, transferred atomically; a recruit can still be enlisted if no suitable gear is available. Armor affects damage, though the vanilla villager renderer does not show it.

The native startup catalog defines these military items; catalog presence does not mean the recruitment selector equips every item. Soldiers use native melee attacks.

| Catalog item | Profile |
| --- | --- |
| Arming sword | One-handed melee |
| Longsword | Two-handed melee |
| Spear | Two-handed melee |
| War bow | Projectile weapon using arrows |
| Mail hauberk | Chest armor |
| Pavise | Deployable shield |

## Commands and orders

Commands target the settlement at the player's position. Army mutations require owner or operator permission. The Army dialog also offers recruitment, dismissal, and individual or group orders.

```text
/settlement army list
/settlement army recruit
/settlement army dismiss <uuid>
/settlement army order <all|uuid> <follow|guard|patrol|return>
```

- **Follow:** follow the issuing player; if unavailable, use the settlement anchor as the goal.
- **Guard:** hold the issuing player's current block inside the settlement and return to it after combat.
- **Patrol:** walk local goals around the settlement anchor.
- **Return:** go to the barracks without engaging enemies.

Follow, Guard and Patrol target nearby hostile mobs within the settlement radius. Orders persist across restart.

## Dismissal and relocation

Dismissal returns equipped items to settlement storage atomically. It fails if there is insufficient storage space or equipment cannot be accessed, leaving the soldier and equipment in place. Dismiss soldiers before unbinding them or relocating their household; relocation checks enlisted family members across dimensions. If reconciliation discovers a foreign-settlement soldier during a household move, it clears enlistment without deleting the equipment. Removing a settlement clears its soldiers' membership and orders but leaves equipment on their bodies.
