//! Settlement dialog text in English and Russian.
//!
//! The edge carries the client's `ClientInformation.language` into the core as
//! [`PlayerInput::SetLocale`](crate::boundary::PlayerInput::SetLocale); [`Lang`]
//! selects Russian for every `ru*` locale and English for everything else.
//! One table, one key per user-visible string: `fill` substitutes `{name}`
//! placeholders, so translated word order and punctuation stay in the table.
//!
//! Game data (item, blueprint, profession and settlement names, certificates'
//! missing-material descriptions) is never translated here.

/// Dialog language chosen from the client locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    En,
    Ru,
}

impl Lang {
    /// `ru`, `ru_ru`, `ru_RU` select Russian; any other or unknown locale is English.
    #[must_use]
    pub(crate) fn of(locale: Option<&str>) -> Self {
        match locale.and_then(|locale| locale.get(..2)) {
            Some(prefix) if prefix.eq_ignore_ascii_case("ru") => Self::Ru,
            _ => Self::En,
        }
    }
}
/// Short task-oriented guide, shared by the edge and settlement dialogs.
#[must_use]
pub fn guide(locale: Option<&str>) -> &'static str {
    text(Lang::of(locale), Key::Guide)
}

/// One-line login hint in the client's language.
#[must_use]
pub fn welcome(locale: Option<&str>) -> &'static str {
    text(Lang::of(locale), Key::Welcome)
}

/// Every user-visible settlement dialog string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    Close,
    Back,
    ViewOnly,
    Denied,
    PositionUnknown,
    NoSettlement,
    AdmissionUnavailable,
    Pages,
    SettlementGone,

    KindHouse,
    KindStorage,
    KindMine,
    KindLumberCamp,
    KindBarracks,
    KindWall,
    KindTower,
    KindGate,
    KindWorkshop,
    CategoryHousing,
    CategoryStorage,
    CategoryProduction,
    CategoryMilitary,
    CategoryFortification,

    TitleSettlement,
    Autonomous,
    Buildings,
    ResidentsJobs,
    Projects,
    ProductionOrders,
    Army,

    MaxLevel,
    ReadyFor,
    More,
    CertifyNew,

    TitleCertify,
    Certify,
    CheckHere,
    InspectHelp,
    InspectOpenAir,
    InspectNotEnclosed,
    InspectTooLarge,
    InspectUnloaded,
    InspectNoFloor,
    InspectNoWorkPoint,
    InspectNoWall,
    InspectNoGate,
    InspectOccupied,
    LabelKind,
    ChooseKind,
    Certified,
    CannotCertify,
    BuildingGone,
    Upgraded,
    CannotUpgrade,

    JobCancelled,
    JobDone,
    JobPaused,
    JobWaiting,
    JobWorking,
    OrderRow,
    Resume,
    Pause,
    Cancel,
    NewOrder,
    NoStation,
    StationRow,
    ChooseStation,
    WhereWork,
    StationGone,
    NoFreeResident,
    TitleNewOrder,
    PickProduct,
    RecipeFacts,
    ExactComponents,
    Quantity,
    Resident,
    FailureBusy,
    FailureNotFound,
    FailureForbidden,
    FailureBlocked,
    FailureUnloaded,
    FailureOther,
    OrderIncomplete,
    WorkshopGone,
    WrongStation,
    NoStand,
    OrderStarted,
    OrderGone,
    OrderUpdated,
    AutoCraft,
    OrderPlanned,
    JobAwaitingStep,
    AutoUnknownRecipe,
    AutoMissing,
    AutoCycle,
    AutoTool,
    AutoStation,
    AutoTooLarge,
    AutoExists,

    NoSoldiers,
    Recruit,
    FollowMe,
    GuardHere,
    Patrol,
    Return,
    Dismiss,
    FollowMeAll,
    GuardHereAll,
    PatrolAll,
    ReturnAll,
    TitleArmy,
    TitleSoldier,
    SoldierButton,
    Recruited,
    Dismissed,
    SoldierGone,
    ChooseOrder,
    Ordered,

    OrderBlueprint,
    ChooseBlueprint,
    PendingOrder,
    PendingBusy,
    PendingMaterials,
    PendingSite,
    CancelPending,
    PendingCancelled,
    Status,

    SettingsBody,
    Rename,
    TitleSettings,
    NewName,
    EnterName,
    Renamed,

    Map,
    MapTerritory,
    MapOutside,
    MapYou,
    MapYouAt,
    MapSiteAt,
    MapBuilding,
    MapWorkshops,
    MapUnpavedRoad,
    MapUnpavedRoads,
    MapPavedRoad,
    MapPavedRoads,
    MapLampSite,
    MapLampSites,
    MapScale,
    MapEmpty,
    Settlement,
    NextStep,
    Help,
    CivilHelp,
    HubBody,
    ClaimReview,
    ClaimPolicy,
    Certificate,
    CertificateCheck,
    ConstructionUpgrade,
    CertificatePolicy,
    RequestPolicy,
    StateChanged,
    SupplyCauseUnavailable,
    SourceMaterialHint,
    NoBlueprintUpgrade,
    GuideClaim,
    GuideRecruit,
    GuideArmour,
    GuideWeapons,
    GuideHousing,
    GuideBattle,
    GuideSupply,
    GuideConstruction,
    MaterialsEstimate,
    MaterialStock,
    HomeMap,
    WorkMap,
    EmptyBuildings,
    EmptyStorage,
    NeedStorage,
    StorageFirst,
    AfterCertified,
    Guide,
    Welcome,
    ConstructionStarted,
    OrderAccepted,
    StockForOrder,
    MissingMaterials,
    MoreMaterialKinds,
    PendingReady,
}

/// English and Russian text for one key.
const fn pair(key: Key) -> (&'static str, &'static str) {
    match key {
        Key::Settlement => ("Management", "Управление"),
        Key::NextStep => ("Next step", "Следующий шаг"),
        Key::Help => ("Help", "Помощь"),
        Key::MaterialsEstimate => ("Materials estimate", "Оценка материалов"),
        Key::MaterialStock => (
            " ×{count} · in storage {stock}\n",
            " ×{count} · на складе {stock}\n",
        ),
        Key::HomeMap => ("Home / bed on map", "Дом / кровать на карте"),
        Key::WorkMap => ("Workplace on map", "Место работы на карте"),
        Key::EmptyBuildings => (
            "No buildings here yet. Order a new one in Construction, or Certify one you built.",
            "Здесь зданий пока нет. Построить новое — «Строительство»; засчитать своё — «Сертифицировать».",
        ),
        Key::EmptyStorage => (
            "Bring materials: Shift-click your own stack.",
            "Принесите материалы: Shift+клик по своему стаку.",
        ),
        Key::NeedStorage => (
            "First, storage: certify a building with a chest, then stock it.",
            "Сначала нужен склад: сертифицируйте постройку с сундуком, затем принесите материалы.",
        ),
        Key::StorageFirst => ("Next: storage needed", "Дальше: нужен склад"),
        Key::AfterCertified => (
            "Next: assign residents, or build the next level and recheck its certificate.",
            "Дальше: назначьте жителей или достройте следующий уровень и проверьте сертификат.",
        ),
        Key::Welcome => (
            "Settlements: /settlement guide",
            "Поселения: /settlement guide",
        ),
        Key::Guide => (
            "1. Found your town: /settlement create Oak. Open management with /settlement.\n2. First, storage: stock your starter chest if a safe spot was found, or place and certify storage. Each concurrent town can receive a chest; removing towns does not renew grants. Shift-click stacks in Storage.\n3. Construction: choose a blueprint, review materials and order. Put missing materials in storage.\n4. Built it yourself? Stand inside, choose its type and Check here. Read every requirement and fix the crosses; Certify when ready. No corner marks needed.\n5. Residents and jobs: place beds and workstations; assign residents to certified workplaces.\n6. Check Construction for progress, Storage for supplies and Map for locations. Back returns to your previous screen.",
            "1. Основать поселение: /settlement create Oak. Управление: /settlement.\n2. Сначала склад: наполните стартовый сундук, если нашлось безопасное место, или поставьте и сертифицируйте свой склад. Каждое одновременное поселение может получить сундук; удаление поселений не возобновляет выдачу. Перенос стаков — Shift+клик.\n3. Строительство: выберите чертёж, проверьте материалы и закажите. Недостающее принесите на склад.\n4. Построили сами? Встаньте внутри, выберите тип и нажмите «Проверить здесь». Исправьте пункты с ✘ и сертифицируйте готовое здание. Углы отмечать не нужно.\n5. Жители и работа: поставьте кровати и рабочие блоки; назначьте жителей в засчитанные мастерские.\n6. Ход работ — в «Строительстве», материалы — в «Складе», расположение — на «Карте». «Назад» вернёт на предыдущий экран.",
        ),
        Key::ConstructionStarted => (
            "Construction started: {blueprint}.",
            "Строительство начато: {blueprint}.",
        ),
        Key::OrderAccepted => ("Order accepted: {blueprint}.", "Заказ принят: {blueprint}."),
        Key::StockForOrder => (
            "Put missing materials in Storage; follow progress in Construction.",
            "Положите недостающие материалы на склад; ход работ смотрите в «Строительстве».",
        ),
        Key::MissingMaterials => ("Missing:", "Не хватает:"),
        Key::MoreMaterialKinds => ("and {n} more item kinds", "и ещё {n} {kind}"),
        Key::PendingReady => ("ready to start", "готов к началу работ"),
        Key::GuideClaim => (
            "Find the village bell → right-click → Claim charter. This is your management entrance.",
            "Найдите колокол деревни → ПКМ → «Принять грамоту». Это вход в управление.",
        ),
        Key::GuideRecruit => (
            "Open Army for recruitment. Need barracks capacity? Open Construction → Barracks.",
            "Откройте «Армия» для найма. Не хватает мест? «Строительство» → «Казарма».",
        ),
        Key::GuideArmour => (
            "The feudal preparation step is Armour: supply real armour through Storage, then issue it to the selected recruit when supported by Army.",
            "Шаг подготовки — «Одеть»: внесите настоящую броню на склад, затем выдайте выбранному новобранцу через доступные действия армии.",
        ),
        Key::GuideWeapons => (
            "Weapons: sword / spear / bow / crossbow. Archers need real arrows; a spear is two-handed. Use Army's supported controls.",
            "Оружие: меч / копьё / лук / арбалет. Лучникам нужны настоящие стрелы; копьё двуручное. Доступные действия — в «Армия».",
        ),
        Key::GuideHousing => (
            "Military housing is an actual free barracks bed, not just reserved capacity. Complete preparation before battle.",
            "Военное жильё — настоящая свободная койка в казарме, а не только место в лимите. Завершите подготовку до боя.",
        ),
        Key::GuideBattle => (
            "Battle preparation: formation → advance / hold / retreat. A window ending is not victory; peace or surrender records the outcome.",
            "В бою: строй → наступать / держать / отступить. Конец окна — не победа; мир или капитуляция записывают исход.",
        ),
        Key::GuideSupply => (
            "Supply voluntarily through Storage. Shift-click your own stack to deposit. Victory grants no right to break or loot others' property.",
            "Снабжение — добровольно через склад. Shift+клик по своему стаку вносит его. Победа не даёт права ломать и грабить чужое.",
        ),
        Key::GuideConstruction => (
            "Building → Order construction: builders fetch materials and place blocks. Construction shows shortages and progress. Recheck certificate only checks blocks already built.",
            "Здание → «Заказать строительство»: строители носят материалы и ставят блоки. «Строительство» показывает нехватку и прогресс. Сертификат проверяет уже построенное.",
        ),
        Key::CivilHelp => (
            "Management: Map, Buildings, Residents and Resources above; Production and Construction below. Click an icon for its labelled page. Back returns without cancelling work.",
            "Управление: вверху Карта, Постройки, Жители и Ресурсы; ниже Производство и Строительство. Клик открывает подписанную страницу. Назад не отменяет работу.",
        ),
        Key::HubBody => (
            "{owner}\nOpen Army to recruit, equip and house soldiers.",
            "{owner}\nНажмите «Армия», чтобы нанять, одеть и поселить солдат",
        ),
        Key::ClaimReview => ("Claim charter", "Принять грамоту"),
        Key::ClaimPolicy => (
            "Claim {name}? You must stand inside this autonomous settlement. Claiming makes you its holder; opening this page changes nothing.",
            "Принять грамоту {name}? Нужно стоять внутри этого автономного поселения. Вы станете его владельцем; открытие этой страницы ничего не меняет.",
        ),
        Key::Certificate => (
            "Recheck certificate L{level}",
            "Проверить сертификат L{level}",
        ),
        Key::CertificateCheck => ("Certificate check", "Проверка сертификата"),
        Key::ConstructionUpgrade => ("Order construction L{level}", "Заказать стройку L{level}"),
        Key::CertificatePolicy => (
            "Checks the building you already built. Nothing is constructed.",
            "Проверка уже построенного: ничего не строится.",
        ),
        Key::RequestPolicy => (
            "Builders will construct the building using storage materials. No blocks are placed or stock debited until the order starts.",
            "Строители построят здание из материалов склада. Блоки не ставятся и склад не списывается, пока заказ не начнётся.",
        ),
        Key::StateChanged => (
            "The target or queued order changed. Review the current state before confirming again.",
            "Цель или очередь изменились. Проверьте текущее состояние перед новым подтверждением.",
        ),
        Key::SupplyCauseUnavailable => (
            "Cause unknown; check storage.",
            "Причина не определена; проверьте склад.",
        ),
        Key::SourceMaterialHint => (
            "Native production input, for example:",
            "Материал для производства, например:",
        ),
        Key::NoBlueprintUpgrade => (
            "No blueprint upgrade; build manually and recheck.",
            "Нет улучшения по чертежу; постройте вручную и проверьте сертификат.",
        ),
        Key::Close => ("Close", "Закрыть"),
        Key::Back => ("Back", "Назад"),
        Key::ViewOnly => ("View only.", "Только просмотр."),
        Key::Denied => (
            "Only the owner or an operator can change this settlement.",
            "Менять это поселение может только владелец или оператор.",
        ),
        Key::PositionUnknown => ("Your position is unknown.", "Ваша позиция неизвестна."),
        Key::NoSettlement => (
            "No settlement here. Stand inside one, or found one with /settlement create <name> [radius].",
            "Здесь нет поселения. Встаньте внутрь или основайте своё: /settlement create <name> [radius].",
        ),
        Key::AdmissionUnavailable => (
            "Settlement command admission is unavailable.",
            "Приём команд поселения недоступен.",
        ),
        Key::Pages => (
            "Pages: main, buildings, map, storage, certify, residents, production, army, projects, settings.",
            "Страницы: main, buildings, map, storage, certify, residents, production, army, projects, settings.",
        ),
        Key::SettlementGone => (
            "That settlement no longer exists.",
            "Это поселение больше не существует.",
        ),

        Key::KindHouse => ("House", "Дом"),
        Key::KindStorage => ("Storage", "Склад"),
        Key::KindMine => ("Mine", "Шахта"),
        Key::KindLumberCamp => ("Lumber camp", "Лесопилка"),
        Key::KindBarracks => ("Barracks", "Казарма"),
        Key::KindWall => ("Wall section", "Секция стены"),
        Key::KindTower => ("Tower", "Башня"),
        Key::KindGate => ("Gate", "Ворота"),
        Key::KindWorkshop => ("{profession} workshop", "Мастерская: {profession}"),
        Key::CategoryHousing => ("Housing", "Жильё"),
        Key::CategoryStorage => ("Storage", "Склад"),
        Key::CategoryProduction => ("Production", "Производство"),
        Key::CategoryMilitary => ("Military", "Военное"),
        Key::CategoryFortification => ("Fortification", "Укрепления"),

        Key::TitleSettlement => ("Settlement {name}", "Поселение {name}"),
        Key::Autonomous => ("Autonomous (unclaimed)", "Автономное (без владельца)"),
        Key::Buildings => ("Buildings", "Постройки"),
        Key::ResidentsJobs => ("Residents & jobs", "Жители и работа"),
        Key::Projects => ("Construction", "Строительство"),
        Key::ProductionOrders => ("Production orders", "Заказы производства"),
        Key::Army => ("Army", "Армия"),

        Key::MaxLevel => (" - max level", " — максимальный уровень"),
        Key::ReadyFor => (" - ready for L{next}", " — готово к L{next}"),
        Key::More => ("(+{n} more)", "(ещё {n})"),
        Key::CertifyNew => ("Certify new building", "Сертифицировать постройку"),

        Key::TitleCertify => ("Certify building", "Сертификация постройки"),
        Key::Certify => ("Certify", "Сертифицировать"),
        Key::CheckHere => ("Check here", "Проверить здесь"),
        Key::InspectHelp => (
            "Stand inside the room, choose its type and Check here. Doors separate rooms.",
            "Встаньте внутри комнаты, выберите тип и нажмите «Проверить здесь». Двери разделяют комнаты.",
        ),
        Key::InspectOpenAir => (
            "No surrounding walls found. Stand inside a building (search limit: 64 blocks).",
            "Стены вокруг не найдены. Встаньте внутри постройки (предел поиска — 64 блока).",
        ),
        Key::InspectNotEnclosed => (
            "No enclosed room found. Close wall openings and stand inside one room.",
            "Замкнутая комната не найдена. Закройте проёмы в стенах и встаньте внутри одной комнаты.",
        ),
        Key::InspectTooLarge => (
            "The room is open or too large: at most 64 blocks per axis and 32768 blocks total. Enclose a smaller room.",
            "Комната не замкнута или слишком большая: до 64 блоков по оси и 32768 блоков всего. Отделите меньшую комнату.",
        ),
        Key::InspectUnloaded => (
            "The building extends into unloaded terrain. Load the surrounding chunks before checking.",
            "Часть постройки не загружена. Загрузите соседние чанки перед проверкой.",
        ),
        Key::InspectNoFloor => (
            "No solid floor found within eight blocks below you. Stand on the room's floor.",
            "Сплошной пол не найден в восьми блоках под вами. Встаньте на пол комнаты.",
        ),
        Key::InspectNoWorkPoint => (
            "No free matching chest or workstation within four blocks. Approach the work area and check again.",
            "В четырёх блоках нет свободного сундука или рабочего блока выбранного типа. Подойдите к рабочей площадке и проверьте снова.",
        ),
        Key::InspectNoWall => (
            "No free wall block within two blocks. Stand beside the wall and check again.",
            "В двух блоках нет свободного блока стены. Встаньте рядом со стеной и проверьте снова.",
        ),
        Key::InspectNoGate => (
            "No free gate door within four blocks. Approach the door in its frame and check again.",
            "В четырёх блоках нет свободной двери ворот. Подойдите к двери в раме и проверьте снова.",
        ),
        Key::InspectOccupied => (
            "This area overlaps another certificate. Stand in a separate work area or inside the existing building of the selected type.",
            "Площадка пересекается с другим сертификатом. Встаньте на отдельной площадке или внутри существующей постройки выбранного типа.",
        ),
        Key::LabelKind => ("Building type", "Тип постройки"),
        Key::ChooseKind => ("Choose a building type.", "Выберите тип постройки."),
        Key::Certified => (
            "Certified {kind} #{id} (level {level}).",
            "Постройка сертифицирована: {kind} #{id} (уровень {level}).",
        ),
        Key::CannotCertify => (
            "Cannot certify {kind}. Missing: {missing}.",
            "Не удалось сертифицировать {kind}. Не хватает: {missing}.",
        ),
        Key::BuildingGone => (
            "That building no longer exists.",
            "Эта постройка больше не существует.",
        ),
        Key::Upgraded => (
            "Building #{id} upgraded to level {level}.",
            "Постройка #{id} улучшена до уровня {level}.",
        ),
        Key::CannotUpgrade => (
            "Cannot upgrade #{id}. Missing: {missing}.",
            "Не удалось улучшить #{id}. Не хватает: {missing}.",
        ),

        Key::JobCancelled => ("cancelled", "отменён"),
        Key::JobDone => ("done", "готово"),
        Key::JobPaused => ("paused", "пауза"),
        Key::JobWaiting => ("waiting: {reason}", "ждёт: {reason}"),
        Key::JobWorking => ("working", "в работе"),
        Key::OrderRow => (
            "{id}: {recipe} {done}/{count} by {worker} - {status}",
            "{id}: {recipe} {done}/{count}, {worker} — {status}",
        ),
        Key::Resume => ("Resume {id}", "Продолжить {id}"),
        Key::Pause => ("Pause {id}", "Пауза {id}"),
        Key::Cancel => ("Cancel {id}", "Отменить {id}"),
        Key::NewOrder => ("New order", "Новый заказ"),
        Key::NoStation => (
            "No crafting table inside a certified workshop. Place a crafting table in a workshop and try again.",
            "В сертифицированной мастерской нет верстака. Поставьте верстак и повторите.",
        ),
        Key::StationRow => (
            "#{id} {kind}: {block} at {at}",
            "#{id} {kind}: {block} в {at}",
        ),
        Key::ChooseStation => ("Choose a workstation", "Выбор рабочего места"),
        Key::WhereWork => (
            "Where should the resident work?",
            "Где должен работать житель?",
        ),
        Key::StationGone => ("That workstation is gone.", "Это рабочее место исчезло."),
        Key::NoFreeResident => (
            "No free resident: bind villagers with /settlement bind, or wait for current orders to finish.",
            "Нет свободных жителей: привяжите жителей через /settlement bind или дождитесь окончания текущих заказов.",
        ),
        Key::TitleNewOrder => ("New production order", "Новый заказ производства"),
        Key::PickProduct => (
            "Select a resident and craft count. Workstation at {at}.",
            "Выберите жителя и число операций. Рабочее место в {at}.",
        ),
        Key::RecipeFacts => (
            "{recipe}\nPer craft — inputs: {inputs}\nOutputs: {outputs}\nDuration: {ticks} ticks",
            "{recipe}\nНа операцию — сырьё: {inputs}\nВыход: {outputs}\nДлительность: {ticks} тактов",
        ),
        Key::ExactComponents => (
            " [exact native components]",
            " [точные нативные компоненты]",
        ),
        Key::Quantity => ("Craft count", "Число операций"),
        Key::Resident => ("Resident", "Житель"),
        Key::FailureBusy => (
            "That resident already has an active order.",
            "У этого жителя уже есть активный заказ.",
        ),
        Key::FailureNotFound => (
            "Resident, workshop or recipe not found.",
            "Житель, мастерская или рецепт не найдены.",
        ),
        Key::FailureForbidden => (
            "That resident does not belong to this settlement.",
            "Этот житель не принадлежит этому поселению.",
        ),
        Key::FailureBlocked => (
            "The workstation is missing or blocked.",
            "Рабочее место отсутствует или заблокировано.",
        ),
        Key::FailureUnloaded => ("The workshop is not loaded.", "Мастерская не загружена."),
        Key::FailureOther => ("Refused: {reason}.", "Отказано: {reason}."),
        Key::OrderIncomplete => (
            "The order form was incomplete.",
            "Форма заказа заполнена не полностью.",
        ),
        Key::WorkshopGone => (
            "That workshop no longer exists.",
            "Эта мастерская больше не существует.",
        ),
        Key::WrongStation => (
            "That recipe needs a different workstation than the one chosen.",
            "Этому рецепту нужно другое рабочее место, не выбранное.",
        ),
        Key::NoStand => (
            "No free standing cell next to that workstation.",
            "Рядом с этим рабочим местом нет свободной клетки.",
        ),
        Key::OrderStarted => (
            "Order {job} started: {item} x{count}.",
            "Заказ {job} начат: {item} ×{count}.",
        ),
        Key::OrderGone => (
            "That order no longer exists.",
            "Этот заказ больше не существует.",
        ),
        Key::OrderUpdated => ("Order {job} updated.", "Заказ {job} обновлён."),
        Key::AutoCraft => ("Craft missing ingredients", "Крафтить недостающее"),
        Key::OrderPlanned => (
            "Order {job} planned: {steps}.",
            "Заказ {job} запланирован: {steps}.",
        ),
        Key::JobAwaitingStep => ("waiting for {job}", "ждёт {job}"),
        Key::AutoUnknownRecipe => ("unknown recipe {recipe}", "неизвестный рецепт {recipe}"),
        Key::AutoMissing => (
            "missing {count} {item}: not in storage and no recipe makes it",
            "не хватает {count} {item}: нет на складе и нет рецепта",
        ),
        Key::AutoCycle => (
            "missing {count} {item}: its recipes form a cycle ({recipes})",
            "не хватает {count} {item}: рецепты образуют цикл ({recipes})",
        ),
        Key::AutoTool => (
            "{recipe} needs the resident to carry {tool}",
            "для {recipe} житель должен нести {tool}",
        ),
        Key::AutoStation => (
            "{recipe} needs {station} inside a certified workshop",
            "для {recipe} нужен {station} в сертифицированной мастерской",
        ),
        Key::AutoTooLarge => (
            "the order needs more than {steps} steps or {portions} crafts per step",
            "заказу нужно больше {steps} шагов или {portions} крафтов на шаг",
        ),
        Key::AutoExists => ("order {job} already exists", "заказ {job} уже существует"),

        Key::NoSoldiers => (
            "No soldiers. Recruit an idle adult; equipment comes from settlement storage.",
            "Солдат нет. Наймите свободного взрослого: снаряжение берётся со склада поселения.",
        ),
        Key::Recruit => ("Recruit soldier", "Нанять солдата"),
        Key::FollowMe => ("Follow me", "Следовать за мной"),
        Key::GuardHere => ("Guard here", "Охранять здесь"),
        Key::Patrol => ("Patrol", "Патруль"),
        Key::Return => ("Return", "Вернуться"),
        Key::Dismiss => ("Dismiss", "Уволить"),
        Key::FollowMeAll => ("Follow me (all)", "Следовать за мной (все)"),
        Key::GuardHereAll => ("Guard here (all)", "Охранять здесь (все)"),
        Key::PatrolAll => ("Patrol (all)", "Патруль (все)"),
        Key::ReturnAll => ("Return (all)", "Вернуться (все)"),
        Key::TitleArmy => ("Army - {name}", "Армия — {name}"),
        Key::TitleSoldier => ("Soldier {id} - {name}", "Солдат {id} — {name}"),
        Key::SoldierButton => ("Soldier {id}", "Солдат {id}"),
        Key::Recruited => ("Recruited {uuid}.", "Нанят {uuid}."),
        Key::Dismissed => ("Dismissed {uuid}.", "Уволен {uuid}."),
        Key::SoldierGone => (
            "That soldier no longer exists.",
            "Этот солдат больше не существует.",
        ),
        Key::ChooseOrder => ("Choose an order.", "Выберите приказ."),
        Key::Ordered => (
            "Ordered {count} soldier(s).",
            "Приказ отдан: солдат {count}.",
        ),

        Key::OrderBlueprint => ("Order: {blueprint}", "Заказ: {blueprint}"),
        Key::ChooseBlueprint => (
            "Choose a blueprint to order.",
            "Выберите чертёж для заказа.",
        ),
        Key::PendingOrder => (
            "Pending #{index}: {blueprint} L{level} · {reason}",
            "В очереди #{index}: {blueprint} ур. {level} · {reason}",
        ),
        Key::PendingBusy => ("busy", "идёт другая стройка"),
        Key::PendingMaterials => ("waiting for materials", "не хватает материалов"),
        Key::PendingSite => ("waiting for a site", "ожидание места для стройки"),
        Key::CancelPending => ("Cancel pending order", "Отменить заказ в очереди"),
        Key::PendingCancelled => ("Pending order cancelled.", "Заказ в очереди отменён."),
        Key::Status => ("Status", "Статус"),

        Key::SettingsBody => (
            "Name: {name}\nRadius: {radius} blocks (grows with population)\nRevision: {revision}",
            "Название: {name}\nРадиус: {radius} блоков (растёт с населением)\nРевизия: {revision}",
        ),
        Key::Rename => ("Rename", "Переименовать"),
        Key::TitleSettings => ("Settings - {name}", "Настройки — {name}"),
        Key::NewName => (
            "New name (letters, digits, - _)",
            "Новое имя (буквы, цифры, - _)",
        ),
        Key::EnterName => ("Enter a name.", "Введите имя."),
        Key::Renamed => ("Renamed to {name}.", "Переименовано в {name}."),

        Key::Map => ("Map", "Карта"),
        Key::MapTerritory => ("Territory, radius {radius}", "Территория, радиус {radius}"),
        Key::MapOutside => ("Outside the settlement", "За пределами поселения"),
        Key::MapYou => ("You", "Вы"),
        Key::MapYouAt => ("You are here: {x} {z}", "Вы здесь: {x} {z}"),
        Key::MapSiteAt => (
            "Construction: {blueprint} L{level}\n{from} .. {to}",
            "Стройка: {blueprint} L{level}\n{from} .. {to}",
        ),
        Key::MapBuilding => (
            "{kind} #{seq} L{level}\n{from} .. {to}",
            "{kind} #{seq} L{level}\n{from} .. {to}",
        ),
        Key::MapWorkshops => ("Workshops", "Мастерские"),
        Key::MapUnpavedRoad => (
            "Street plan: unpaved road\n{from} .. {to}",
            "План улиц: дорога без покрытия\n{from} .. {to}",
        ),
        Key::MapUnpavedRoads => ("Road (planned)", "Дорога (план)"),
        Key::MapPavedRoad => (
            "Paved road\n{from} .. {to}",
            "Дорога (замощена)\n{from} .. {to}",
        ),
        Key::MapPavedRoads => ("Road (paved)", "Дорога (замощена)"),
        Key::MapLampSite => (
            "Street plan: lamp post site\n{at}",
            "План улиц: место фонаря\n{at}",
        ),
        Key::MapLampSites => ("Lamp (planned)", "Фонарь (план)"),
        Key::MapScale => (
            "1 cell = {scale}×{scale} blocks, north up",
            "1 клетка = {scale}×{scale} {blocks}, север сверху",
        ),
        Key::MapEmpty => ("No buildings yet.", "Построек пока нет."),
    }
}

/// Text for `key` in `lang`.
#[must_use]
pub(crate) fn text(lang: Lang, key: Key) -> &'static str {
    let (english, russian) = pair(key);
    match lang {
        Lang::En => english,
        Lang::Ru => russian,
    }
}

/// Russian noun form for a count: `one` for 1, 21, 101; `few` for 2-4, 22-24;
/// `many` for 0, 5-20, 25-30, 111-114 and every other number.
#[must_use]
pub(crate) fn plural(
    n: u64,
    one: &'static str,
    few: &'static str,
    many: &'static str,
) -> &'static str {
    let last_two = n % 100;
    let last = n % 10;
    if (11..=14).contains(&last_two) {
        many
    } else if last == 1 {
        one
    } else if (2..=4).contains(&last) {
        few
    } else {
        many
    }
}

/// Count of item kinds the receipt left out: "and N more item kinds" /
/// «и ещё N вид|вида|видов».
#[must_use]
pub(crate) fn more_material_kinds(lang: Lang, n: usize) -> String {
    fill(
        text(lang, Key::MoreMaterialKinds),
        &[
            ("n", &n.to_string()),
            ("kind", plural(n as u64, "вид", "вида", "видов")),
        ],
    )
}

/// Map cell size: "1 cell = S×S blocks" / «1 клетка = S×S блок|блока|блоков».
#[must_use]
pub(crate) fn map_scale(lang: Lang, scale: i32) -> String {
    let scale = scale.max(0);
    fill(
        text(lang, Key::MapScale),
        &[
            ("scale", &scale.to_string()),
            (
                "blocks",
                plural(
                    u64::try_from(scale).unwrap_or_default(),
                    "блок",
                    "блока",
                    "блоков",
                ),
            ),
        ],
    )
}

/// Names shared by workshop certificates and resident cards.
pub(crate) fn profession(
    lang: Lang,
    profession: crate::villager::VillagerProfession,
) -> &'static str {
    use crate::villager::VillagerProfession as P;
    let (en, ru) = match profession {
        P::None => ("Unemployed", "Безработный"),
        P::Nitwit => ("Nitwit", "Бездельник"),
        P::Armorer => ("Armorer", "Бронник"),
        P::Butcher => ("Butcher", "Мясник"),
        P::Cartographer => ("Cartographer", "Картограф"),
        P::Cleric => ("Cleric", "Священник"),
        P::Farmer => ("Farmer", "Фермер"),
        P::Fisherman => ("Fisherman", "Рыбак"),
        P::Fletcher => ("Fletcher", "Лучник"),
        P::Leatherworker => ("Leatherworker", "Кожевник"),
        P::Librarian => ("Librarian", "Библиотекарь"),
        P::Mason => ("Mason", "Каменщик"),
        P::Shepherd => ("Shepherd", "Пастух"),
        P::Toolsmith => ("Toolsmith", "Инструментальщик"),
        P::Weaponsmith => ("Weaponsmith", "Оружейник"),
        P::Miner => ("Miner", "Шахтёр"),
        P::Lumberjack => ("Lumberjack", "Лесоруб"),
        P::Builder => ("Builder", "Строитель"),
        P::Soldier => ("Soldier", "Солдат"),
    };
    match lang {
        Lang::En => en,
        Lang::Ru => ru,
    }
}

pub(super) fn discovery_refusal(
    lang: Lang,
    reason: super::building_discovery::Refusal,
) -> &'static str {
    use super::building_discovery::Refusal;
    text(
        lang,
        match reason {
            Refusal::OpenAir => Key::InspectOpenAir,
            Refusal::NotEnclosed => Key::InspectNotEnclosed,
            Refusal::TooLarge => Key::InspectTooLarge,
            Refusal::Unloaded => Key::InspectUnloaded,
            Refusal::NoFloor => Key::InspectNoFloor,
            Refusal::NoWorkPoint => Key::InspectNoWorkPoint,
            Refusal::NoWall => Key::InspectNoWall,
            Refusal::NoGate => Key::InspectNoGate,
            Refusal::Occupied => Key::InspectOccupied,
        },
    )
}

/// Certificate geometry counters and the action that satisfies each counter.
pub(crate) fn requirement(lang: Lang, code: &str) -> (&str, &'static str) {
    let (en, ru, en_action, ru_action) = match code {
        "footprint" => (
            "floor area",
            "площадь пола",
            "complete the solid floor",
            "достройте сплошной пол",
        ),
        "height" => (
            "height",
            "высота",
            "increase the building height",
            "увеличьте высоту постройки",
        ),
        "beds" => (
            "beds",
            "кровати",
            "add the missing complete beds",
            "добавьте недостающие кровати целиком",
        ),
        "roof" => (
            "roof",
            "крыша",
            "close the holes in the roof",
            "закройте отверстия в крыше",
        ),
        "enclosure" => (
            "perimeter walls",
            "стены по периметру",
            "close gaps in the walls",
            "закройте дыры в стенах",
        ),
        "entrance" => (
            "doors",
            "дверь",
            "add the missing doors",
            "добавьте недостающие двери",
        ),
        "workstations" => (
            "workstations",
            "рабочие блоки",
            "add the missing workstations",
            "добавьте недостающие рабочие блоки",
        ),
        "containers" => (
            "chests/barrels",
            "сундуки/бочки",
            "add the missing chests or barrels",
            "добавьте недостающие сундуки или бочки",
        ),
        "wall length" => (
            "wall length",
            "длина стены",
            "extend the wall",
            "удлините стену",
        ),
        "wall height" => (
            "wall height",
            "высота стены",
            "raise the wall",
            "нарастите стену",
        ),
        "continuous wall" => (
            "continuous wall",
            "непрерывная стена",
            "close gaps in the wall",
            "закройте разрывы стены",
        ),
        "gate frame" => (
            "gate frame",
            "рама ворот",
            "complete the gate frame",
            "достройте раму ворот",
        ),
        _ => return (code, text(lang, Key::InspectHelp)),
    };
    match lang {
        Lang::En => (en, en_action),
        Lang::Ru => (ru, ru_action),
    }
}

/// Substitute `{name}` placeholders; unknown placeholders stay verbatim.
#[must_use]
pub(crate) fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        let Some(close) = tail.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let key = &tail[..close];
        match args.iter().find(|(name, _)| *name == key) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(key);
                out.push('}');
            }
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_prefix_selects_russian_and_everything_else_is_english() {
        assert_eq!(Lang::of(Some("ru_ru")), Lang::Ru);
        assert_eq!(Lang::of(Some("ru")), Lang::Ru);
        assert_eq!(Lang::of(Some("ru_RU")), Lang::Ru);
        assert_eq!(Lang::of(Some("en_us")), Lang::En);
        assert_eq!(Lang::of(Some("de_de")), Lang::En);
        assert_eq!(Lang::of(Some("")), Lang::En);
        assert_eq!(Lang::of(None), Lang::En);
    }

    #[test]
    fn fill_replaces_known_placeholders_only() {
        assert_eq!(
            fill("Owner: {owner} ({n})", &[("owner", "Mayor")]),
            "Owner: Mayor ({n})"
        );
        assert_eq!(fill("no holes", &[]), "no holes");
        assert_eq!(fill("{a}{a}", &[("a", "x")]), "xx");
    }

    #[test]
    fn known_navigation_strings_are_translated() {
        assert_eq!(text(Lang::En, Key::Back), "Back");
        assert_eq!(text(Lang::Ru, Key::Back), "Назад");
        assert_eq!(text(Lang::Ru, Key::Army), "Армия");
        assert_eq!(text(Lang::Ru, Key::ClaimReview), "Принять грамоту");
    }

    #[test]
    fn russian_plural_picks_one_few_many() {
        for (n, expected) in [
            (0, "видов"),
            (1, "вид"),
            (2, "вида"),
            (4, "вида"),
            (5, "видов"),
            (11, "видов"),
            (12, "видов"),
            (14, "видов"),
            (21, "вид"),
            (22, "вида"),
            (25, "видов"),
            (101, "вид"),
            (111, "видов"),
            (112, "видов"),
        ] {
            assert_eq!(plural(n, "вид", "вида", "видов"), expected, "n = {n}");
        }
    }

    #[test]
    fn counters_agree_with_their_counts_in_both_languages() {
        assert_eq!(more_material_kinds(Lang::Ru, 1), "и ещё 1 вид");
        assert_eq!(more_material_kinds(Lang::Ru, 2), "и ещё 2 вида");
        assert_eq!(more_material_kinds(Lang::Ru, 5), "и ещё 5 видов");
        assert_eq!(more_material_kinds(Lang::Ru, 21), "и ещё 21 вид");
        assert_eq!(more_material_kinds(Lang::En, 2), "and 2 more item kinds");
        assert_eq!(map_scale(Lang::Ru, 1), "1 клетка = 1×1 блок, север сверху");
        assert_eq!(map_scale(Lang::Ru, 2), "1 клетка = 2×2 блока, север сверху");
        assert_eq!(
            map_scale(Lang::Ru, 8),
            "1 клетка = 8×8 блоков, север сверху"
        );
        assert_eq!(map_scale(Lang::En, 8), "1 cell = 8×8 blocks, north up");
    }
}
