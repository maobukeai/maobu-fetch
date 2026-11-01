// Read-only syntax audit: compares top-level IPC keys with the local Tauri command signatures.
import fs from "node:fs/promises";
import ts from "typescript";

const rust=await fs.readFile(new URL("../../src-tauri/src/lib.rs",import.meta.url),"utf8");
const progressCommands=await fs.readFile(new URL("../../src-tauri/src/progress_window/commands.rs",import.meta.url),"utf8");
const completionCommands=await fs.readFile(new URL("../../src-tauri/src/progress_window/completion_commands.rs",import.meta.url),"utf8");
const typescript=await fs.readFile(new URL("../../src/api.ts",import.meta.url),"utf8");
const progressTypescript=await fs.readFile(new URL("../../src/components/progress/progress-api.ts",import.meta.url),"utf8");
const camel=value=>value.replace(/_([a-z])/g,(_,letter)=>letter.toUpperCase());
const commands=new Map();
const resultTypes=new Map();
const pattern=/#\[tauri::command([^\]]*)\](?:\s*#\[[^\]]*\])*\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*\(/g;
const commandSources=rust+"\n"+progressCommands+"\n"+completionCommands;
const stateAliases=[...commandSources.matchAll(/^type\s+(\w+)(?:<[^=]+>)?\s*=\s*(?:tauri::)?State\s*</gm)].map(match=>match[1]);
for(const match of commandSources.matchAll(pattern)){
  let start=match.index+match[0].length,position=start,depth=1;
  while(depth){const char=commandSources[position++];if(char==="(")depth++;else if(char===")")depth--;}
  const parameters=[];let current="",angle=0,paren=0;
  for(const char of commandSources.slice(start,position-1)){
    if(char==="<")angle++;else if(char===">")angle=Math.max(0,angle-1);
    if(char==="(")paren++;else if(char===")")paren--;
    if(char===","&&angle===0&&paren===0){parameters.push(current);current="";}else current+=char;
  }
  if(current.trim())parameters.push(current);
  const args=parameters.map(parameter=>/^\s*(\w+)\s*:\s*([\s\S]+)$/.exec(parameter)).filter(Boolean)
    .filter(([, ,type])=>!/(?:\bState\s*<|\bAppHandle\b|\bWebviewWindow\b|\bWindow\b)/.test(type)&&!stateAliases.some(alias=>new RegExp("\\b"+alias+"\\s*<").test(type)))
    .map(([,name,type])=>({name:match[1].includes("snake_case")?name:camel(name),optional:/^Option\s*</.test(type.trim())}));
  commands.set(match[2],args);
  resultTypes.set(match[2],commandSources.slice(position,commandSources.indexOf("{",position)).trim());
}
let source;
const observations=[],problems=[],unresolved=[];
const registeredBlock=/\.invoke_handler\(tauri::generate_handler!\[([\s\S]*?)\]\)/.exec(rust);
if(!registeredBlock)throw new Error("Cannot locate the actual native command registry");
const registered=new Set(registeredBlock[1].split(",").map(name=>name.trim().split("::").at(-1)).filter(Boolean));
function visit(node){
  if(ts.isCallExpression(node)&&["call","invoke"].includes(node.expression.getText(source))&&ts.isStringLiteral(node.arguments[0])){
    const command=node.arguments[0].text,expected=commands.get(command),object=node.arguments[1];
    if(!registered.has(command))problems.push({command,reason:"Command is not registered with invoke_handler"});
    if(!expected)problems.push({command,reason:"No matching annotated Rust command"});
    if(command.startsWith("progress_window_")&&node.typeArguments?.[0]?.getText(source)==="ProgressWindowSnapshot"&&!/->\s*Result\s*<\s*(?:\w+::)*Snapshot\s*,/.test(resultTypes.get(command)??"")){
      problems.push({command,reason:"The native result does not match the declared progress snapshot"});
    }
    else if(object&&!ts.isObjectLiteralExpression(object))unresolved.push({command,reason:"Dynamic IPC argument object"});
    else{
      const keys=(object?.properties??[]).filter(property=>ts.isPropertyAssignment(property)||ts.isShorthandPropertyAssignment(property))
        .map(property=>property.name.getText(source).replace(/^["']|["']$/g,""));
      const missing=expected.filter(argument=>!argument.optional&&!keys.includes(argument.name)).map(argument=>argument.name);
      const misnamed=keys.filter(key=>!expected.some(argument=>argument.name===key)&&expected.some(argument=>argument.name===camel(key))&&!keys.includes(camel(key)));
      if(missing.length||misnamed.length)problems.push({command,missing,misnamed});
      if(object?.properties.some(ts.isSpreadAssignment))unresolved.push({command,reason:"IPC object contains spread"});
      observations.push({command,keys});
    }
  }
  ts.forEachChild(node,visit);
}
for(const [name,code] of [["api.ts",typescript],["progress-api.ts",progressTypescript]]){
  source=ts.createSourceFile(name,code,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
  visit(source);
}
console.log(JSON.stringify({nativeCommands:commands.size,checkedCalls:observations.length,problems,unresolved},null,2));
if(problems.length||unresolved.length)process.exitCode=1;
