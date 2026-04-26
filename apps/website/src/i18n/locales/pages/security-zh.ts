export const zhSecurity = {
	"security.meta.title": "安全与合规 | Flow-Like",
	"security.meta.description":
		"基于 Rust 构建，具备内存安全、RBAC、完整审计追踪、数据主权和开源透明度。企业级安全，无需企业级的复杂性。",
	"security.hero.tagline": "安全优先",
	"security.hero.headline": "可以",
	"security.hero.headline.highlight": "验证的安全",
	"security.hero.description":
		"Flow-Like 是开源的。每一行代码都可以审计。将 Rust 的内存安全特性与基于角色的访问控制、静态加密和完整的审计追踪相结合。",
	"security.hero.cta": "查看源代码",
	"security.hero.cta.report": "报告漏洞",
	"security.arch.tagline": "安全架构",
	"security.arch.headline": "纵深防御",
	"security.arch.description": "多层安全防护——从语言运行时到部署边界。",
	"security.arch.rust.title": "内存安全运行时",
	"security.arch.rust.desc":
		"整个执行引擎使用 Rust 编写——在编译时消除缓冲区溢出、释放后使用和数据竞争。无垃圾回收器暂停，无运行时意外。",
	"security.arch.wasm.title": "沙箱化扩展",
	"security.arch.wasm.desc":
		"自定义节点在具有基于能力安全模型的 WASM 沙箱中运行。除非明确授权，否则无法访问文件系统或网络。恶意代码无法逃离沙箱。",
	"security.arch.rbac.title": "基于角色的访问控制",
	"security.arch.rbac.desc":
		"针对工作流、节点、密钥和部署的细粒度权限。在组织、团队或项目级别分配角色。默认执行最小权限原则。",
	"security.arch.encryption.title": "全面加密",
	"security.arch.encryption.desc":
		"传输中的数据使用 TLS 1.3。密钥、凭证和敏感工作流数据使用 AES-256 静态加密。通过您的 KMS 或我们的 KMS 管理密钥。",
	"security.arch.audit.title": "完整审计追踪",
	"security.arch.audit.desc":
		"每次工作流执行、配置变更和访问事件都会记录时间戳、用户身份和完整上下文。可导出到您的 SIEM 或合规工具。",
	"security.arch.supply.title": "供应链安全",
	"security.arch.supply.desc":
		"所有依赖项均通过 SBOM 追踪。第三方许可证持续审计。依赖项更新在发布前经过 CI 测试。",
	"security.data.tagline": "数据主权",
	"security.data.headline": "您的数据，您做主",
	"security.data.description":
		"Flow-Like 从不要求您的数据离开您的基础设施。在本地、在您的 VPC 中或在桌面上运行——除非您选择加入，否则零遥测。",
	"security.data.local.title": "本地优先架构",
	"security.data.local.desc":
		"桌面应用完全支持离线工作。无需云依赖。您的工作流、数据和密钥保留在您的机器上。",
	"security.data.selfhost.title": "自托管部署",
	"security.data.selfhost.desc":
		"在您自己的云或本地基础设施中部署 Flow-Like。支持 Docker、Kubernetes 和裸机部署。",
	"security.data.residency.title": "数据驻留控制",
	"security.data.residency.desc":
		"选择数据处理和存储的位置。通过部署级别的控制满足 GDPR、CCPA 和监管要求。",
	"security.compliance.tagline": "合规与透明",
	"security.compliance.headline": "为受监管行业而生",
	"security.compliance.description":
		"从医疗到金融再到政府——Flow-Like 提供受监管环境所要求的控制措施。",
	"security.compliance.gdpr.title": "GDPR 就绪",
	"security.compliance.gdpr.desc":
		"数据删除工作流、同意管理和处理记录。可随时请求数据删除。",
	"security.compliance.soc.title": "SOC 2 控制",
	"security.compliance.soc.desc":
		"访问控制、变更管理和监控与 SOC 2 信任服务标准保持一致。",
	"security.compliance.open.title": "开源透明",
	"security.compliance.open.desc":
		"每个依赖项、每个许可证、每一行代码——均可公开审计。查看完整的第三方声明。",
	"security.compliance.sbom.title": "SBOM 可用",
	"security.compliance.sbom.desc":
		"每次发布均生成软件物料清单。包含许可证和漏洞数据的完整依赖树。",
	"security.cta.headline": "有安全方面的问题？",
	"security.cta.description":
		"我们的安全团队随时准备讨论您的需求。如需报告漏洞，请使用我们的负责任披露流程。",
	"security.cta.button": "联系安全团队",
	"security.cta.thirdparty": "查看第三方声明",
} as const;
