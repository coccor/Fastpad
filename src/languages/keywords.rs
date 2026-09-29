//! Lexilla keyword sets (`SCI_SETKEYWORDS`), one const per language and set. Lexilla matches SQL,
//! PowerShell and Batch words after lower-casing, so those lists are lower case.

macro_rules! js_words {
    () => {
        "async await break case catch class const continue debugger default delete do else export \
         extends false finally for from function get if import in instanceof let new null of \
         return set static super switch this throw true try typeof undefined var void while with \
         yield"
    };
}

pub(crate) const JSON: &str = "true false null";
pub(crate) const JSON_LD: &str = "@id @context @type @value @language @graph @list @set @reverse \
    @index @base @vocab @container @nest @prefix @version @protected @propagate @import @included \
    @direction @json @none";
pub(crate) const YAML: &str =
    "true false yes no null on off True False Yes No Null On Off TRUE FALSE YES NO NULL ON OFF ~";
pub(crate) const TOML: &str = "true false inf nan";

pub(crate) const JAVASCRIPT: &str = js_words!();
pub(crate) const TYPESCRIPT: &str = concat!(
    js_words!(),
    " abstract accessor any as asserts bigint boolean declare enum global implements infer \
     interface is keyof module namespace never number object override private protected public \
     readonly require satisfies string symbol type unique unknown"
);
pub(crate) const JS_GLOBALS: &str = "Array ArrayBuffer BigInt Boolean Date Error JSON Map Math \
    Number Object Promise Proxy Reflect RegExp Set String Symbol WeakMap WeakSet console document \
    globalThis window process";

macro_rules! c_words {
    () => {
        "auto break case char const continue default do double else enum extern float for goto if \
         inline int long register restrict return short signed sizeof static struct switch \
         typedef union unsigned void volatile while _Alignas _Alignof _Atomic _Bool _Complex \
         _Generic _Noreturn _Static_assert _Thread_local alignas alignof bool false true nullptr \
         static_assert thread_local typeof"
    };
}

pub(crate) const C: &str = c_words!();
pub(crate) const C_TYPES: &str = "size_t ssize_t ptrdiff_t intptr_t uintptr_t int8_t int16_t \
    int32_t int64_t uint8_t uint16_t uint32_t uint64_t wchar_t FILE NULL";
pub(crate) const CPP: &str = concat!(
    c_words!(),
    " and and_eq asm bitand bitor catch char8_t char16_t char32_t class co_await co_return \
     co_yield compl concept consteval constexpr constinit const_cast decltype delete \
     dynamic_cast explicit export final friend mutable namespace new noexcept not not_eq operator \
     or or_eq override private protected public reinterpret_cast requires static_cast template \
     this throw try typeid typename using virtual xor xor_eq"
);
pub(crate) const CPP_TYPES: &str = "std string wstring string_view vector array map unordered_map \
    set unordered_set optional variant span unique_ptr shared_ptr weak_ptr size_t int8_t int16_t \
    int32_t int64_t uint8_t uint16_t uint32_t uint64_t";
pub(crate) const CSHARP: &str = "abstract add alias and as async await base bool break by byte \
    case catch char checked class const continue decimal default delegate descending do double \
    dynamic else enum equals event explicit extern false file finally fixed float for foreach \
    from get global goto group if implicit in init int interface internal into is join let lock \
    long managed nameof namespace new nint not notnull nuint null object on operator or orderby \
    out override params partial private protected public readonly record ref remove required \
    return sbyte scoped sealed select set short sizeof stackalloc static string struct switch \
    this throw true try typeof uint ulong unchecked unmanaged unsafe ushort using value var \
    virtual void volatile when where while with yield";
pub(crate) const CSHARP_TYPES: &str = "Console DateTime Dictionary Exception Guid HashSet \
    IEnumerable IList List Math Span String Task TimeSpan";

pub(crate) const PYTHON: &str = "False None True and as assert async await break case class \
    continue def del elif else except finally for from global if import in is lambda match \
    nonlocal not or pass raise return try type while with yield";
pub(crate) const PYTHON_BUILTINS: &str = "abs all any bool bytes callable chr classmethod dict \
    dir enumerate filter float format getattr hasattr hash id input int isinstance issubclass \
    iter len list map max min next object open ord print property range repr reversed round set \
    setattr slice sorted staticmethod str sum super tuple type zip self cls";

pub(crate) const RUST: &str = "as async await break const continue crate dyn else enum extern \
    false fn for if impl in let loop match mod move mut pub ref return self Self static struct \
    super trait true type union unsafe use where while";
pub(crate) const RUST_TYPES: &str = "bool char f32 f64 i8 i16 i32 i64 i128 isize str u8 u16 u32 \
    u64 u128 usize String Vec Option Result Box Rc Arc Some None Ok Err";

pub(crate) const BASH: &str = "alias break case cd continue declare do done echo elif else esac \
    eval exec exit export false fi for function if in local printf pwd read readonly return \
    select set shift source test then time trap true typeset unset until while";
pub(crate) const BATCH: &str = "assoc break call cd chdir chcp choice cls copy date defined del \
    dir do echo else endlocal equ erase errorlevel exist exit for ftype geq goto gtr if in leq \
    lss md mkdir mklink move neq not nul path pause popd prompt pushd rd rem ren rename rmdir \
    set setlocal shift start time title type ver verify vol";
pub(crate) const POWERSHELL: &str = "begin break catch class continue data default do \
    dynamicparam else elseif end enum exit filter finally for foreach function hidden if in \
    param process return static switch throw trap try until using while";
pub(crate) const POWERSHELL_CMDLETS: &str = "add-content add-type convertfrom-json \
    convertto-json copy-item export-modulemember foreach-object format-list format-table \
    get-childitem get-command get-content get-date get-help get-item get-location get-member \
    get-process get-service group-object import-module invoke-command invoke-expression \
    invoke-restmethod invoke-webrequest join-path measure-object move-item new-item new-object \
    out-file out-null out-string read-host remove-item resolve-path select-object set-content \
    set-item set-location sort-object split-path start-process start-sleep stop-process \
    test-path where-object write-error write-host write-output write-verbose write-warning";
pub(crate) const POWERSHELL_ALIASES: &str =
    "cat cd cp del dir echo fl ft gc gci gi iex irm iwr ls mv ni ri rm sc select sort where";

pub(crate) const SQL: &str = "add all alter and any as asc begin between by case cast check \
    column commit constraint create cross database declare default delete desc distinct drop \
    else end exec execute exists foreign from full function grant group having if in index inner \
    insert into is join key left like limit merge not null offset on or order outer over \
    partition primary procedure references replace return returns right rollback row rows \
    select set table then top transaction trigger truncate union unique update using values view \
    when where while with";
pub(crate) const SQL_TYPES: &str = "avg bigint binary bit blob boolean char coalesce count date \
    datetime datetime2 decimal double float getdate int integer isnull max min money nchar now \
    nullif numeric nvarchar real smallint sum text time timestamp tinyint uniqueidentifier \
    varbinary varchar xml";
