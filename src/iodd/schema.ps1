$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$script:issues = New-Object 'System.Collections.Generic.List[object]'
$script:digests = New-Object 'System.Collections.Generic.List[object]'
$script:cache = @{}
$script:total = 0
$script:truncated = $false
$report = [ordered]@{ passed = $false; engine = 'Windows PowerShell / System.Xml XSD 1.0'; runtime = [Environment]::Version.ToString(); xml_sha256 = ''; root_schema = ''; schemas = @(); diagnostics = @(); diagnostics_truncated = $false }

function Digest([byte[]]$bytes) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant() }
    finally { $hash.Dispose() }
}

function Read-Bounded([string]$path) {
    $stream = [IO.File]::OpenRead($path)
    $memory = New-Object IO.MemoryStream
    try {
        if ($stream.Length -gt 8388608) { throw 'XML or schema exceeds 8 MiB' }
        $buffer = New-Object byte[] 8192
        while (($count = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            if ($memory.Length + $count -gt 8388608) { throw 'XML or schema exceeds 8 MiB' }
            $memory.Write($buffer, 0, $count)
        }
        return ,$memory.ToArray()
    } finally { $stream.Dispose(); $memory.Dispose() }
}

function Reader-Settings {
    $settings = New-Object Xml.XmlReaderSettings
    $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
    $settings.XmlResolver = $null
    $settings.MaxCharactersInDocument = 8388608
    return $settings
}

function Issue([string]$severity, [string]$message, [int]$line, [int]$position) {
    if ($script:issues.Count -ge 100) { $script:truncated = $true; return }
    if ($message.Length -gt 2048) { $message = $message.Substring(0, 2048) }
    $script:issues.Add([ordered]@{ severity = $severity; message = $message; line = $line; position = $position })
}

$handler = [Xml.Schema.ValidationEventHandler] {
    param($sender, $event)
    Issue $event.Severity.ToString() $event.Message $event.Exception.LineNumber $event.Exception.LinePosition
}

function Load-Schema([string]$path) {
    $path = [IO.Path]::GetFullPath($path)
    if (-not $path.StartsWith($script:root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Schema reference escapes the selected schema directory' }
    $check = $path
    while ($check.Length -ge $script:root.Length) {
        if (([IO.File]::GetAttributes($check) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Schema references through reparse points are unsupported' }
        if ($check -eq $script:root) { break }
        $check = [IO.Path]::GetDirectoryName($check)
    }
    if ($script:cache.ContainsKey($path)) { return $script:cache[$path] }
    if ($script:cache.Count -ge 64) { throw 'Schema closure exceeds 64 files' }
    $bytes = Read-Bounded $path
    $script:total += $bytes.Length
    if ($script:total -gt 33554432) { throw 'Schema closure exceeds 32 MiB' }
    $memory = New-Object IO.MemoryStream(,$bytes)
    $uri = New-Object Uri $path
    $reader = [Xml.XmlReader]::Create($memory, (Reader-Settings), $uri.AbsoluteUri)
    try { $schema = [Xml.Schema.XmlSchema]::Read($reader, $handler) }
    finally { $reader.Dispose(); $memory.Dispose() }
    if ($null -eq $schema) { throw 'Schema could not be parsed' }
    $script:cache[$path] = $schema
    $script:digests.Add([ordered]@{ name = $path.Substring($script:root.Length + 1).Replace('\', '/'); sha256 = (Digest $bytes) })
    foreach ($reference in $schema.Includes) {
        if ([string]::IsNullOrWhiteSpace($reference.SchemaLocation)) { throw 'Schema imports require an explicit local schemaLocation' }
        $next = New-Object Uri $uri, $reference.SchemaLocation
        if (-not $next.IsFile -or $next.IsUnc -or $next.Query -or $next.Fragment) { throw 'Only local schema references without query or fragment are allowed' }
        $reference.Schema = Load-Schema $next.LocalPath
    }
    return $schema
}

try {
    $request = [IO.File]::ReadAllText($env:IOLINK_XSD_REQUEST) | ConvertFrom-Json
    $schemaPath = [IO.Path]::GetFullPath($request.schema)
    $script:root = [IO.Path]::GetDirectoryName($schemaPath)
    $report.root_schema = [IO.Path]::GetFileName($schemaPath)
    $schemaUri = New-Object Uri $schemaPath
    if (-not $schemaUri.IsFile -or $schemaUri.IsUnc) { throw 'Root schema must be a local file' }
    $drive = New-Object IO.DriveInfo ([IO.Path]::GetPathRoot($schemaPath))
    if ($drive.DriveType -eq [IO.DriveType]::Network) { throw 'Root schema must be on a local drive' }
    $bytes = Read-Bounded $request.xml
    $report.xml_sha256 = Digest $bytes
    $schema = Load-Schema $schemaPath
    if ($schema.TargetNamespace -ne 'http://www.io-link.com/IODD/2010/10') { throw 'Root schema must target the IODD 1.1 namespace' }
    $schemas = New-Object Xml.Schema.XmlSchemaSet
    $schemas.XmlResolver = $null
    $schemas.add_ValidationEventHandler($handler)
    [void]$schemas.Add($schema)
    $schemas.Compile()
    if (-not $schemas.IsCompiled) { throw 'Schema set did not compile' }
    $settings = Reader-Settings
    $settings.ValidationType = [Xml.ValidationType]::Schema
    $settings.Schemas = $schemas
    # Document-supplied schema locations and inline schemas never participate.
    $settings.ValidationFlags = [Xml.Schema.XmlSchemaValidationFlags]::ProcessIdentityConstraints -bor [Xml.Schema.XmlSchemaValidationFlags]::AllowXmlAttributes -bor [Xml.Schema.XmlSchemaValidationFlags]::ReportValidationWarnings
    $settings.add_ValidationEventHandler($handler)
    $memory = New-Object IO.MemoryStream(,$bytes)
    $reader = [Xml.XmlReader]::Create($memory, $settings)
    try { while ($reader.Read()) {} }
    finally { $reader.Dispose(); $memory.Dispose() }
    $report.passed = $script:issues.Count -eq 0 -and -not $script:truncated
} catch {
    Issue 'Error' $_.Exception.Message 0 0
} finally {
    $report.schemas = @($script:digests.ToArray())
    $report.diagnostics = @($script:issues.ToArray())
    $report.diagnostics_truncated = $script:truncated
    $json = ConvertTo-Json -InputObject $report -Depth 8 -Compress
    [IO.File]::WriteAllText($env:IOLINK_XSD_RESPONSE, $json, (New-Object Text.UTF8Encoding $false))
}
