local process = require("@lune/process")
local fs = require("@lune/fs")
local stdio = require("@lune/stdio")
local serde = require("@lune/serde")

local project_root = nil
local path_components = string.split(string.gsub(process.cwd, "\\", "/"), "/")
if path_components[#path_components] == "" then
    table.remove(path_components)
end

local function in_lockfile(lockfile)
    if not lockfile.graph then
        return false
    end
    
    for _, versions in lockfile.graph do
        for _, node in versions do
            if node.direct and node.direct[1] == "{alias}" then
                return true
            end
        end
    end
    
    return false
end

for i = #path_components, 1, -1 do
    local path = table.concat(path_components, "/", 1, i)
    if not fs.isFile(path .. "/{MANIFEST_FILE_NAME}") then
        continue
    end
    
    if project_root == nil then
        project_root = path
    end
    
    if project_root and fs.isFile(path .. "/{LOCKFILE_FILE_NAME}") then
        local lockfile = serde.decode("toml", fs.readFile(path .. "/{LOCKFILE_FILE_NAME}"))
        if not lockfile.workspace then
            continue
        end
        
        local search_for = string.gsub(project_root, path, "")
        if string.sub(search_for, 1, 1) == "/" then
            search_for = string.sub(search_for, 2)
        end
        
        if search_for == "" then
            if in_lockfile(lockfile) then
                break
            end
            
            continue
        end
        
        for _, targets in lockfile.workspace do
            for _, member_path in targets do
                local path_normalized = string.gsub(member_path, "\\", "/")
                if path_normalized == search_for and in_lockfile(lockfile) then
                    project_root = path
                    break
                end
            end
        end
    end
end

if project_root ~= nil then
    for _, packages_folder in {{ {all_folders} }} do
        local path = `{{project_root}}/{{packages_folder}}/{alias}.bin.luau`
        
        if fs.isFile(path) then
            require(path)
            return
        end
    end
end

stdio.ewrite(stdio.color("red") .. "binary `{alias}` not found. are you in the right directory?" .. stdio.color("reset") .. "\n")
process.exit(1)