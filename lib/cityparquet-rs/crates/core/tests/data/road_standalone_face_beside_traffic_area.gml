<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (robustness).                      -->
<!-- A Road with LoD3 geometry twice over: a TrafficArea defining face "A",    -->
<!-- and a standalone tran:lod3MultiSurface that names A by xlink AND carries   -->
<!-- a face "B" no traffic area covers. One LoD must yield one geometry, and   -->
<!-- B must survive it — untyped, since nothing says what it is.               -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:tran="http://www.opengis.net/citygml/transportation/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<tran:Road gml:id="the-road">
			<tran:trafficArea>
				<tran:TrafficArea gml:id="lane">
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="A">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList srsDimension="3">0 0 0 10 0 0 10 10 0 0 10 0 0 0 0</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:lod3MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember xlink:href="#A"/>
					<gml:surfaceMember>
						<gml:Polygon gml:id="B">
							<gml:exterior>
								<gml:LinearRing>
									<gml:posList srsDimension="3">10 0 0 20 0 0 20 10 0 10 10 0 10 0 0</gml:posList>
								</gml:LinearRing>
							</gml:exterior>
						</gml:Polygon>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod3MultiSurface>
		</tran:Road>
	</cityObjectMember>
</CityModel>
