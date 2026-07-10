use bitflags::bitflags;

bitflags! {
    /// Represents a set of flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct RolePermissions: i64 {
        const Owner             =   0b00000000_00000000_00000000_00000001;
        const Admin             =   0b00000000_00000000_00000000_00000010;
        const ReadTeam          =   0b00000000_00000000_00000000_00000100;
        const ReadRoles         =   0b00000000_00000000_00000000_00001000;
        const ReadFiles         =   0b00000000_00000000_00000000_00010000;
        const WriteFiles        =   0b00000000_00000000_00000000_00100000;
        const InvokeApi         =   0b00000000_00000000_00000000_01000000;
        const WriteMeta         =   0b00000000_00000000_00000000_10000000;
        const ReadBoards        =   0b00000000_00000000_00000001_00000000;
        const ExecuteBoards     =   0b00000000_00000000_00000010_00000000;
        const WriteBoards       =   0b00000000_00000000_00000100_00000000;
        const ListEvents        =   0b00000000_00000000_00001000_00000000;
        const ReadEvents        =   0b00000000_00000000_00010000_00000000;
        const ExecuteEvents     =   0b00000000_00000000_00100000_00000000;
        const WriteEvents       =   0b00000000_00000000_01000000_00000000;
        const ReadLogs          =   0b00000000_00000000_10000000_00000000;
        const ReadAnalytics     =   0b00000000_00000001_00000000_00000000;
        const ReadConfig        =   0b00000000_00000010_00000000_00000000;
        const WriteConfig       =   0b00000000_00000100_00000000_00000000;
        const ReadTemplates     =   0b00000000_00001000_00000000_00000000;
        const WriteTemplates    =   0b00000000_00010000_00000000_00000000;
        const ReadCourses       =   0b00000000_00100000_00000000_00000000;
        const WriteCourses      =   0b00000000_01000000_00000000_00000000;
        const ReadWidgets       =   0b00000000_10000000_00000000_00000000;
        const WriteWidgets      =   0b00000001_00000000_00000000_00000000;
        const WriteRoutes       =   0b00000010_00000000_00000000_00000000;
        /// Database-only read access: query/list the app's databases without
        /// being able to read its files. `ReadFiles` also implies database
        /// read access for backwards compatibility.
        const ReadDatabase      =   0b00000100_00000000_00000000_00000000;
        /// Database-only write access, without file write access.
        /// `WriteFiles` also implies database write access.
        const WriteDatabase     =   0b00001000_00000000_00000000_00000000;
    }
}

pub fn has_role_permission(permissions: &RolePermissions, permission: RolePermissions) -> bool {
    permissions.contains(permission)
        || permissions.contains(RolePermissions::Admin)
        || permissions.contains(RolePermissions::Owner)
}
