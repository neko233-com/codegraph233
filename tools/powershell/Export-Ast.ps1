# Trusted parser helper. Analyzed input is JSON on stdin and is never executed.
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 or newer required' }
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$inputData = [Console]::In.ReadToEnd() | ConvertFrom-Json
$parseTokens = $null; $parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseInput([string]$inputData.source, [ref]$parseTokens, [ref]$parseErrors)
$symbols = [Collections.Generic.List[object]]::new()
$references = [Collections.Generic.List[object]]::new()
$byAst = @{}
$module = [string]$inputData.module
function Add-Symbol($node, $kind, $name, $qual, $owner, [int]$arity) {
    $key = "$qual#$kind@$($node.Extent.StartLineNumber)"
    $signature = ($node.Extent.Text -split "`r?`n")[0]
    if ($signature.Length -gt 512) { $signature = $signature.Substring(0,512) }
    $symbols.Add(@{key=$key;lang='powershell';kind=$kind;name=$name;qualname=$qual;namespace=$module;container=$owner;start_line=$node.Extent.StartLineNumber;end_line=$node.Extent.EndLineNumber;signature=$signature;param_count=$arity;module=$module})
    $byAst["$($node.GetType().Name):$($node.Extent.StartOffset)"] = $key
    return $key
}
function Parent-Class($node) {
    for ($p=$node.Parent; $null-ne $p; $p=$p.Parent) { if ($p -is [Management.Automation.Language.TypeDefinitionAst]) { return $p.Name } }
    return ''
}
function Parent-Function($node) {
    for ($p=$node.Parent; $null-ne $p; $p=$p.Parent) {
        $key = "$($p.GetType().Name):$($p.Extent.StartOffset)"
        if ($byAst.ContainsKey($key)) { return $byAst[$key] }
    }
    return $moduleKey
}
$moduleKey = Add-Symbol $ast 'module' '<module>' "${module}::<module>" '' -1
$declarations = $ast.FindAll({ param($n) $n -is [Management.Automation.Language.TypeDefinitionAst] -or $n -is [Management.Automation.Language.FunctionDefinitionAst] -or $n -is [Management.Automation.Language.FunctionMemberAst] -or $n -is [Management.Automation.Language.PropertyMemberAst] }, $true)
foreach ($node in $declarations) {
    $ownerName = Parent-Class $node
    $owner = if ($ownerName) { "$module.$ownerName" } else { '' }
    if ($node -is [Management.Automation.Language.TypeDefinitionAst]) {
        $kind = if ($node.IsEnum) { 'enum' } else { 'class' }
        $null = Add-Symbol $node $kind $node.Name "$module.$($node.Name)" '' -1
    } elseif ($node -is [Management.Automation.Language.PropertyMemberAst]) {
        $null = Add-Symbol $node 'field' $node.Name "$owner.$($node.Name)" $owner -1
    } elseif ($node -is [Management.Automation.Language.FunctionMemberAst]) {
        $kind = if ($node.IsConstructor) { 'constructor' } else { 'method' }
        $null = Add-Symbol $node $kind $node.Name "$owner.$($node.Name)" $owner @($node.Parameters).Count
    } elseif ($node.Parent -isnot [Management.Automation.Language.FunctionMemberAst]) {
        $parameters = if ($node.Parameters) { @($node.Parameters).Count } elseif ($null -ne $node.Body.ParamBlock) { @($node.Body.ParamBlock.Parameters).Count } else { 0 }
        $null = Add-Symbol $node 'function' $node.Name "$module.$($node.Name)" '' $parameters
    }
}
$calls = $ast.FindAll({param($n) $n -is [Management.Automation.Language.CommandAst] -or $n -is [Management.Automation.Language.InvokeMemberExpressionAst]},$true)
foreach ($node in $calls) {
    $receiver=''; $kind='calls'; $arity=-1
    if ($node -is [Management.Automation.Language.CommandAst]) {
        $name=$node.GetCommandName()
        if (-not $name) { $name=$node.CommandElements[0].Extent.Text; $kind='dynamic-call' }
    } else {
        if ($node.Member -is [Management.Automation.Language.StringConstantExpressionAst]) { $name=$node.Member.Value }
        else { $name=$node.Member.Extent.Text; $kind='dynamic-call' }
        $arity=@($node.Arguments).Count
        if ($node.Expression -is [Management.Automation.Language.TypeExpressionAst]) { $receiver=$node.Expression.TypeName.FullName }
        elseif ($node.Expression -is [Management.Automation.Language.VariableExpressionAst] -and $node.Expression.VariablePath.UserPath -eq 'this') { $receiver=Parent-Class $node }
    }
    $references.Add(@{src_key=(Parent-Function $node);name=[string]$name;kind=$kind;receiver_type=[string]$receiver;namespace_hint=$module;line=$node.Extent.StartLineNumber;arity=$arity})
}
$errors=@($parseErrors | Select-Object -First 32 | ForEach-Object { "PowerShell parser $($_.ErrorId) at $($_.Extent.StartLineNumber):$($_.Extent.StartColumnNumber)" })
@{format='codegraph233.powershell-ast/v1';rel_path=$inputData.rel;root=$inputData.root;lang='powershell';module=$module;symbols=@($symbols);refs=@($references);errors=$errors;parser_version=$PSVersionTable.PSVersion.ToString()} | ConvertTo-Json -Depth 12 -Compress
