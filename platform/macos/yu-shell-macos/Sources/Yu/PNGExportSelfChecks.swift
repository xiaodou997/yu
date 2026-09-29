import AppKit
import ImageIO
import CryptoKit
import Darwin
import YuStorageFFI

/// Production snapshot, layout, raster, and publication paths on owned files.
@MainActor
func runPNGExportSelfCheck(input: String, directory: String) -> Never {
    let fm=FileManager.default, root=URL(fileURLWithPath: directory, isDirectory: true)
    var rows:[[String:Any]]=[], owns=false
    func require(_ v:Bool,_ s:String)throws {if !v {throw NSError(domain:"Yu.PNG.Check",code:1,userInfo:[NSLocalizedDescriptionKey:s])}}
    func report(_ passed:Bool,_ error:String?=nil){
        var data:[String:Any]=["passed":passed,"cases":rows,"evidence_layer":"native"]
        if let error{data["error"]=error}
        if let bytes=try? JSONSerialization.data(withJSONObject:data,options:[.prettyPrinted,.sortedKeys]) {if owns{try? bytes.write(to:root.appendingPathComponent("report.json"))};print(String(decoding:bytes,as:UTF8.self))}
    }
    func dimensions(_ url:URL)throws->[Int]{
        guard let src=CGImageSourceCreateWithURL(url as CFURL,nil),CGImageSourceGetCount(src)==1,
              let props=CGImageSourceCopyPropertiesAtIndex(src,0,nil) as? [CFString:Any],
              let w=props[kCGImagePropertyPixelWidth] as? Int,let h=props[kCGImagePropertyPixelHeight] as? Int,
              CGImageSourceCreateImageAtIndex(src,0,nil) != nil else {throw NSError(domain:"Yu.PNG.Decode",code:1)}
        return [w,h]
    }
    do {
        try require(!fm.fileExists(atPath:root.path),"Refusing existing PNG evidence")
        try fm.createDirectory(at:root,withIntermediateDirectories:false);owns=true
        let inputURL=URL(fileURLWithPath:input), original=try Data(contentsOf:inputURL)
        var normalSizes:[[Int]]=[]
        for mode in ["light-1x","light-2x","dark-1x","split-cancel","split-confirm","split-table","oversize-table","directory-exists","source-alias","cancel"] {
            let work=root.appendingPathComponent(mode);try fm.createDirectory(at:work,withIntermediateDirectories:false)
            let segmented=mode.hasPrefix("split")
            let large=segmented||mode=="directory-exists"
            let path:URL
            if mode=="split-table" {
                path=work.appendingPathComponent("split-table.md")
                let body=(0..<150).map{n in "<tr><td rowspan=\"2\">GROUP-\(n)</td><td>CELL-\(n)-A</td></tr><tr><td>CELL-\(n)-B</td></tr>"}.joined()
                let source="# PNG跨段合并表格\n\n<table><thead><tr><th>组</th><th>内容</th></tr></thead><tbody>\(body)</tbody></table>\n\nPNG-TABLE-END\n"
                try Data(source.utf8).write(to:path,options:.withoutOverwriting)
            } else if mode=="oversize-table" {
                path=work.appendingPathComponent("oversize-table.md")
                let rows=(1..<200).map{n in "<tr><td>ROW-\(n)</td></tr>"}.joined()
                let source="# PNG超大单元素\n\n<table><tbody><tr><td rowspan=\"200\">OVERSIZE-GROUP</td><td>ROW-0</td></tr>\(rows)</tbody></table>\n"
                try Data(source.utf8).write(to:path,options:.withoutOverwriting)
            } else if large {
                path=work.appendingPathComponent("long.md")
                let source="# PNG分段\n\n"+(0..<700).map{"第\($0)段 SEGMENT-LINE-\($0) 中文完整内容。\n\n"}.joined()+"PNG-LONG-END\n"
                try Data(source.utf8).write(to:path,options:.withoutOverwriting)
            } else {path=inputURL}
            let bridge=try StorageBridge(path:path.path)
            try bridge.setSelection(NSRange(location:(bridge.source as NSString).length,length:0))
            _=try bridge.insertText("\nPNG-UNSAVED-SNAPSHOT\n")
            let frozen=bridge.source, target=work.appendingPathComponent("output.png"), parts=work.appendingPathComponent("output-images")
            let old=Data("OLD-PNG-KEEP".utf8)
            if mode=="source-alias"{try fm.linkItem(at:path,to:target)}else{try old.write(to:target,options:.withoutOverwriting)}
            let oldBytes=try Data(contentsOf:target)
            if mode=="directory-exists" {try fm.createDirectory(at:parts,withIntermediateDirectories:false);try old.write(to:parts.appendingPathComponent("keep.txt"))}
            let scale=mode=="light-2x"||large||mode=="oversize-table" ? 2 : 1
            var config:[String:Any]=["exportFormat":"png","title":"PNG固定测试","referenceDay":20724,"width":800,"scale":scale,"replaceExisting":true,"fontSize":16]
            if mode=="dark-1x"{config["dark"]=true;config["foreground"]=UInt32(0xe8eaedff);config["background"]=UInt32(0x202124ff);config["link"]=UInt32(0x8ab4f8ff)}
            let task=try bridge.beginHTMLExport(to:target,config:config);defer{task.cancel()}
            try bridge.setSelection(NSRange(location:0,length:0));_=try bridge.insertText("PNG-LATER-NOT-IN-SNAPSHOT\n")
            try bridge.setSelectionEndpoints(anchorUTF16:8,focusUTF16:1,affinity:0)
            let source=bridge.source,rev=bridge.revision,dirty=bridge.state.dirty
            if mode=="cancel"{task.cancel()}
            var submitted=false,final:NativeHTMLExportTask.Status?,splitSeen=false
            let deadline=Date().addingTimeInterval(180)
            while Date()<deadline {
                let status=try task.status()
                if ["completed","completed_with_warnings","failed","cancelled"].contains(status.phase){final=status;break}
                if status.phase=="split" {
                    splitSeen=true;try require(!fm.fileExists(atPath:parts.path)&&Data(contentsOf:target)==oldBytes,"Split published without consent")
                    if mode=="split-cancel"{task.cancel()}else if !submitted{try task.commit(allowWarnings:false);submitted=true}
                }else if status.phase=="ready" && !submitted{try task.commit(allowWarnings:false);submitted=true}
                else if status.phase=="warnings"{throw NSError(domain:"Yu.PNG.Warnings",code:1,userInfo:[NSLocalizedDescriptionKey:status.warnings.joined(separator:"\n")])}
                Thread.sleep(forTimeInterval:0.01)
            }
            guard let final else {throw NSError(domain:"Yu.PNG.Timeout",code:1)}
            let success=["light-1x","light-2x","dark-1x","split-confirm","split-table"].contains(mode)
            try require(final.phase==(success ? "completed" : (["split-cancel","cancel"].contains(mode) ? "cancelled" : "failed")),"\(mode): \(final.phase) \(final.message)")
            var sizes:[[Int]]=[],hashes:[String]=[]
            if success {
                let urls=large ? try fm.contentsOfDirectory(at:parts,includingPropertiesForKeys:nil).filter{$0.pathExtension=="png"}.sorted{$0.lastPathComponent<$1.lastPathComponent} : [target]
                for (index,url) in urls.enumerated(){if large{try require(url.lastPathComponent==String(format:"part-%03d.png",index+1),"Wrong segment order")};sizes.append(try dimensions(url));hashes.append(SHA256.hash(data:try Data(contentsOf:url)).map{String(format:"%02x",$0)}.joined())}
                try require(sizes==final.pngSizes,"Actual PNG dimensions differ from preflight")
                if mode.hasPrefix("light"){normalSizes.append(sizes[0])}
                if large{try require(splitSeen&&sizes.count>1&&Data(contentsOf:target)==oldBytes,"Split target protection failed")}
                if mode=="split-table"{try require(sizes.count>1,"Merged table did not exercise segment boundaries")}
            }else{try require(try Data(contentsOf:target)==oldBytes,"Failure or cancel changed old target")}
            if mode=="oversize-table"{try require(final.message.contains("合并组")||final.message.contains("放入"),"Oversize table did not fail for the indivisible group")}
            if mode=="split-cancel"{try require(splitSeen && !fm.fileExists(atPath:parts.path),"Split cancellation left output")}
            if mode=="directory-exists"{try require(try fm.contentsOfDirectory(atPath:parts.path)==["keep.txt"],"Existing directory was merged")}
            try require(bridge.source==source&&bridge.revision==rev&&bridge.state.dirty==dirty&&bridge.selectionEndpoints.anchorUTF16==8&&bridge.selectionEndpoints.focusUTF16==1,"PNG changed editor identity")
            _=try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_UNDO));try require(bridge.source==frozen,"PNG damaged undo")
            _=try bridge.executeCommand(UInt8(YU_STORAGE_COMMAND_REDO));try require(bridge.source==source,"PNG damaged redo")
            try require(!(try fm.contentsOfDirectory(atPath:work.path)).contains{$0.hasPrefix(".yu-export-")},"Temporary output remained")
            rows.append(["id":mode,"passed":true,"phase":final.phase,"sizes":sizes,"sha256":hashes])
        }
        try require(normalSizes.count==2&&normalSizes[1]==normalSizes[0].map{$0*2},"2x dimensions are not doubled")
        try require(try Data(contentsOf:inputURL)==original,"Disk input changed")
        report(true);exit(0)
    }catch{report(false,error.localizedDescription);exit(1)}
}
