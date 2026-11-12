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
    TipCorners,
    Corner1,
    TipCorner1,
    Corner2,
    TipCorner2,
    LabelKind,
    LabelFrom,
    LabelTo,
    LabelExtra,
    ChooseKind,
    BadCorners,
    InvalidCorners,
    Certified,
    CannotCertify,
    BuildingGone,
    Upgraded,
    CannotUpgrade,
    CornerSet,

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
    Checklist,
    ChecklistUnavailable,
    Neighbours,
    NeighboursUnavailable,
    GuideClaim,
    GuideChecklist,
    GuideRecruit,
    GuideArmour,
    GuideWeapons,
    GuideHousing,
    GuideWar,
    GuideBattle,
    GuideSupply,
    GuideConstruction,
    MaterialsEstimate,
}

/// English and Russian text for one key.
const fn pair(key: Key) -> (&'static str, &'static str) {
    match key {
        Key::Settlement => ("Settlement", "Поселение"),
        Key::NextStep => ("Next step", "Следующий шаг"),
        Key::Help => ("Help", "Помощь"),
        Key::MaterialsEstimate => ("Materials estimate", "Оценка материалов"),
        Key::Neighbours => ("Neighbours & war", "Соседи и война"),
        Key::NeighboursUnavailable => (
            "Neighbour and war management is not available in this build.",
            "Управление соседями и войной в этой сборке недоступно.",
        ),
        Key::GuideClaim => (
            "Find the village bell → right-click → Claim charter. This is your management entrance.",
            "Найдите колокол деревни → ПКМ → «Принять грамоту». Это вход в управление.",
        ),
        Key::GuideChecklist => (
            "Fief checklist belongs to the native progression system. If unavailable, this UI does not invent readiness.",
            "Список «До удела» принадлежит системе развития. Если он недоступен, интерфейс не выдумывает готовность.",
        ),
        Key::GuideRecruit => (
            "Open Army for the available recruitment controls. Need barracks capacity? Open Projects → Barracks.",
            "Откройте «Армия» для доступного найма. Не хватает мест? «Проекты» → «Казарма».",
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
        Key::GuideWar => (
            "Only the charter holder approves a war's cause, objectives and window. No damage is lawful during the grace period. Controls require native war support.",
            "Причину, цели и окно войны утверждает держатель грамоты. До конца отсрочки урон запрещён. Действия требуют поддержки системы войны.",
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
            "Building → Order construction: builders fetch materials and place blocks. Projects shows shortages and progress. Recheck certificate only checks blocks already built.",
            "Здание → «Заказать стройку»: строители носят материалы и ставят блоки. «Проекты» показывают нехватку и прогресс. Сертификат проверяет уже построенное.",
        ),
        Key::CivilHelp => (
            "Settlement dashboard: Map, Buildings, Residents and Resources above; Production and Projects below. Click an icon for its labelled page. Back returns without cancelling work.",
            "Панель поселения: вверху Карта, Постройки, Жители и Ресурсы; ниже Производство и Проекты. Клик открывает подписанную страницу. Назад не отменяет работу.",
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
            "Check blocks already built.\nbuilders will not build.\nThese are geometry requirements,\nnot a shopping bill.",
            "Проверка уже построенных блоков.\nстроители не будут строить.\nЭто требования к геометрии,\nне список покупок.",
        ),
        Key::RequestPolicy => (
            "Request; queues if busy or short.\nBefore admission: estimated materials.\nNo blocks are placed\nand no stock debited by this click.",
            "Если занято/нехватка — в очередь.\nДо приёма: материалы — оценка.\nЭтот клик не ставит блоки\nи не списывает запас.",
        ),
        Key::StateChanged => (
            "The target or queued order changed. Review the current state before confirming again.",
            "Цель или очередь изменились. Проверьте текущее состояние перед новым подтверждением.",
        ),
        Key::SupplyCauseUnavailable => (
            "Native supply cause unavailable. Check storage and production; no source is inferred.",
            "Причина снабжения недоступна. Проверьте склад и производство; источник не выводится.",
        ),
        Key::SourceMaterialHint => (
            "Native production input, for example:",
            "Материал для производства, например:",
        ),
        Key::NoBlueprintUpgrade => (
            "No blueprint upgrade; build manually and recheck.",
            "Нет улучшения по чертежу; постройте вручную и проверьте сертификат.",
        ),
        Key::Checklist => ("Fief checklist", "До удела"),
        Key::ChecklistUnavailable => (
            "Fief progression is not available in this server state. Civil counts are not a promotion test.",
            "Развитие до удела недоступно в этом состоянии сервера. Гражданская статистика не является проверкой титула.",
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
        Key::Projects => ("Projects", "Стройка"),
        Key::ProductionOrders => ("Production orders", "Заказы производства"),
        Key::Army => ("Army", "Армия"),

        Key::MaxLevel => (" - max level", " — максимальный уровень"),
        Key::ReadyFor => (" - ready for L{next}", " — готово к L{next}"),
        Key::More => ("(+{n} more)", "(ещё {n})"),
        Key::CertifyNew => ("Certify new building", "Сертифицировать постройку"),

        Key::TitleCertify => ("Certify building", "Сертификация постройки"),
        Key::Certify => ("Certify", "Сертифицировать"),
        Key::TipCorners => (
            "Corners are two opposite corners of the finished building, inclusive. Height adds blocks above the higher corner.",
            "Углы — две противоположные вершины готовой постройки, включительно. Высота добавляет блоки выше верхнего угла.",
        ),
        Key::Corner1 => ("Corner 1 here", "Угол 1 = моя позиция"),
        Key::TipCorner1 => (
            "Stand in a corner block and press this; walk to the opposite corner, then /settlement menu certify and press Corner 2 here.",
            "Встаньте в угловой блок и нажмите; перейдите в противоположный угол, затем /settlement menu certify и нажмите «Угол 2 = моя позиция».",
        ),
        Key::Corner2 => ("Corner 2 here", "Угол 2 = моя позиция"),
        Key::TipCorner2 => (
            "Stand in the opposite corner block and press this.",
            "Встаньте в противоположный угловой блок и нажмите.",
        ),
        Key::LabelKind => ("Building type", "Тип постройки"),
        Key::LabelFrom => ("Corner 1 (x y z)", "Угол 1 (x y z)"),
        Key::LabelTo => ("Corner 2 (x y z)", "Угол 2 (x y z)"),
        Key::LabelExtra => ("Height above corners", "Высота над углами"),
        Key::ChooseKind => ("Choose a building type.", "Выберите тип постройки."),
        Key::BadCorners => (
            "Corners must be three whole numbers: x y z.",
            "Углы задаются тремя целыми числами: x y z.",
        ),
        Key::InvalidCorners => ("Invalid corners.", "Неверные углы."),
        Key::Certified => (
            "Certified {kind} #{id} (level 1).",
            "Постройка сертифицирована: {kind} #{id} (уровень 1).",
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
        Key::CornerSet => ("Corner {n} = {at}.", "Угол {n} = {at}."),

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
            "Pending: {blueprint} L{level} — {reason}",
            "В очереди: {blueprint} ур. {level} — {reason}",
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
            "1 клетка = {scale}×{scale} блоков, север сверху",
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
}
