use bitflags::bitflags;

bitflags! {
    /// Represents a set of flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct GlobalPermission: i64 {
        const Admin             =   0b00000000_00000000_00000001;
        const ReadPublishing    =   0b00000000_00000000_00000010;
        const WritePublishing   =   0b00000000_00000000_00000100;
        const ReadProfile       =   0b00000000_00000000_00001000;
        const WriteProfile      =   0b00000000_00000000_00010000;
        const ReadApps          =   0b00000000_00000000_00100000;
        const WriteApps         =   0b00000000_00000000_01000000;
        const WriteLandingPage  =   0b00000000_00000000_10000000;
        const ReadTransactions  =   0b00000000_00000001_00000000;
        const WriteTransactions =   0b00000000_00000010_00000000;
        const WriteBits         =   0b00000000_00000100_00000000;
        const ReadSolutions     =   0b00000000_00001000_00000000;
        const WriteSolutions    =   0b00000000_00010000_00000000;
        const ManagePackages    =   0b00000000_00100000_00000000;
        const ReadCourses       =   0b00000000_01000000_00000000;
        const WriteCourses      =   0b00000000_10000000_00000000;
        const ReadLogs          =   0b00000001_00000000_00000000;
    }
}
